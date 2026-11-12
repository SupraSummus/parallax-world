// compositor.ts in WGSL, for cells' reliefs of whole voxels (bake's
// cell_relief), or their whole voxels as splats, and their see-through
// voxels as splats, drawing only the clusters of them that may show: each
// cluster names its chunk's slot in `chunks`, which gives its face and
// plane relative to the camera. Per frame, cull_drawn lists draws of the
// tiles of columns (or clusters of whole splats) that showed last frame,
// and tile_vertex (splat_vertex) draws them; their depth is reduced to a
// pyramid of farthest depths (reduce_depth, reduce), and cull_rest tests
// every tile against it: those that showed are kept for the next frame,
// and those newly showing drawn now. Then the see-through splats in front
// of them, nearest first (veil_fragment), over the picture.

// A face's own rays, for pictures drawn flat (flat_vertex).
enable clip_distances;

struct FaceView {
  // Columns: the camera's right, up and forward axes, in the face's coordinates.
  view: mat3x3f,
  // Its axis in world coordinates, times √2: (±1, ±1, 0) in some order.
  axis: vec4f,
}

struct Globals {
  viewport: vec2f, // CSS pixels (points)
  resolution: vec2f, // device pixels
  focal: f32,
  fog_distance: f32,
  whole: f32, // how whole voxels are drawn: 0 reliefs' tiles, 1 splats, 2 flat pictures (FLAT)
  sky_fog: f32,
  sky_top: vec4f,
  horizon: vec4f,
  sky_view: mat3x3f, // the camera's axes in world coordinates
  faces: array<FaceView, 12>,
}

struct Chunk {
  origin: vec2f, // top-left corner of what is drawn, relative to the camera, face coordinates
  distance: f32, // of the centre plane, along the face's axis
  face: u32,
  level: f32, // see depth_of
  tint: u32, // RGBA to paint its voxels in, shaded as they are; 0 for their own
  half_layer: f32, // a half layer of its grid: the unit of its columns' and splats' depths
  clip: f32, // see veil_vertex
  // Drawn flat (compositor.rs's Flat): its picture's q from its origin at
  // scale · q + offset, its plane past its own, z0 + gx x + gy y at q =
  // (x, y) (w unused).
  scale: vec2f,
  offset: vec2f,
  plane: vec4f,
}
const FLAT = 2.0;

// A tile of a chunk's relief (GpuCluster), TILE × TILE of its columns from
// `first`, by rows, the centre of its first column `base`; or see-through
// splats of it near each other on its plane, `first` and the next `count`
// (low byte of `slot_count`), the widest's half width `wide`. Its chunk's
// slot (`slot_count` above the low byte), the box of their outlines along
// the axis relative to the chunk's origin, whether any showed in the last
// frame, and how far past the plane they are, nearest and farthest.
struct Cluster {
  min: vec2f,
  max: vec2f,
  base: vec2f,
  first: u32,
  slot_count: u32,
  visible: u32,
  near: f32,
  far: f32,
  wide: f32,
}
const TILE = 4;

// A draw: a tile's quad, an instance of tile_vertex; or a cluster's
// splats, instances of splat_vertex or veil_vertex.
struct DrawArgs {
  vertices: u32,
  instances: u32,
  first_vertex: u32,
  first_instance: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> chunks: array<Chunk>;
// Drawing: the tiles, and their columns (GpuColumn), each its front's depth
// past the plane and its pillar's run, in half layers, as i16s, low first,
// and its colour; 0, none.
@group(0) @binding(2) var<storage, read> tiles: array<Cluster>;
@group(0) @binding(3) var<storage, read> columns: array<vec2u>;
// Culling: the frame's chunks' clusters (candidate), and a draw for each
// to draw.
@group(0) @binding(4) var<storage, read_write> clusters: array<Cluster>;
@group(0) @binding(5) var<storage, read> candidates: array<vec4u>;
@group(0) @binding(6) var<storage, read_write> draws: array<DrawArgs>;
@group(0) @binding(7) var<storage, read_write> drawn: atomic<u32>;
@group(0) @binding(8) var farthest: texture_2d<f32>;
// The see-through clusters, as candidates: per chunk, finest level first and
// nearest first in each, their draws in that order.
@group(0) @binding(9) var<storage, read> veiled: array<vec4u>;
@group(0) @binding(16) var<storage, read_write> veil_draws: array<DrawArgs>;
// The pyramid: level 0 from the depth buffer, each next from the last.
@group(0) @binding(10) var depth_in: texture_depth_2d;
@group(0) @binding(11) var level_in: texture_2d<f32>;
@group(0) @binding(12) var level_out: texture_storage_2d<r32float, write>;
// Showing the frame.
@group(0) @binding(13) var picture: texture_2d<f32>;
// The see-through splats over each other (veil_fragment).
@group(0) @binding(14) var veil: texture_2d<f32>;

// The depth of a voxel of a chunk of `level`, its site `distance` from the
// eye: finer levels in front (lod.rs), and within a level the nearer site,
// which, voxels being the lattice's Voronoi cells, is the one a ray meets
// first. Level L's depths are [2^(L - 64), 2^(L - 63)), HALFWAY halfway:
// any distance in the view's scale keeps f32's steps far below a voxel.
const HALFWAY = 1000.0;
fn depth_of(level: f32, distance: f32) -> f32 {
  return exp2(level - 64.0) * (1.0 + distance / (distance + HALFWAY));
}

struct Splat {
  @builtin(position) position: vec4f,
  @location(0) corner: vec2f,
  @location(1) @interpolate(flat) slant: f32,
  // Its alpha its voxel's: of the rays through its site, how many it stops.
  @location(2) @interpolate(flat) colour: vec4f,
  @location(3) @interpolate(flat) face: u32,
}

fn ray_of(view: mat3x3f, position: vec4f) -> vec3f {
  let s = position.xy / globals.resolution - 0.5;
  return view * vec3f(vec2f(s.x, -s.y) * globals.viewport / globals.focal, 1.0);
}

// A splat instance (GpuSplat): its hexagon's centre relative to its chunk's
// origin, half its width (its height √2 times that) and its depth past the
// plane, and its colour; drawn at its depth, as much wider as the ray to it
// slants across (compositor.ts). Its chunk's slot comes as the first
// vertex's index over 4.
@vertex
fn splat_vertex(@builtin(vertex_index) vertex: u32, @location(0) splat: vec4f, @location(1) paint: vec4f) -> Splat {
  let site = splat_site(vertex, splat);
  let size = vec2f(splat.z * (1.0 + abs(site.x / site.z)), splat.z * sqrt(2.0));
  return placed(vertex, splat, paint, size);
}

// A see-through splat: its voxel's share of a field over its layer (see
// veil_fragment), but none where the next finer level draws its cell
// (`finer`: how far past the plane that cell's site is, in half layers;
// lod.rs, Lod::finer_reach): in front, that level holds what it does.
@vertex
fn veil_vertex(@builtin(vertex_index) vertex: u32, @location(0) splat: vec4f, @location(1) paint: vec4f, @location(2) finer: f32) -> Splat {
  var out = placed(vertex, splat, paint, 2.0 * splat.z * vec2f(1.0, sqrt(2.0)));
  if globals.whole == FLAT {
    // In front of the pictures or behind them as their depths say (flat_depth).
    let c = chunks[vertex >> 2u];
    out.position.z = flat_depth(c.level, dot(globals.faces[c.face].view[2], splat_site(vertex, splat))) * out.position.w;
  }
  if finer <= chunks[vertex >> 2u].clip {
    out.position = vec4f(2.0, 2.0, 2.0, 1.0);
  }
  return out;
}

// Its site relative to the camera, in its face's coordinates.
fn splat_site(vertex: u32, splat: vec4f) -> vec3f {
  let c = chunks[vertex >> 2u];
  return vec3f(c.origin + splat.xy, c.distance + splat.w * c.half_layer);
}

// A splat drawn as a box `size` from its site each way.
fn placed(vertex: u32, splat: vec4f, paint: vec4f, size: vec2f) -> Splat {
  let c = chunks[vertex >> 2u];
  let face = globals.faces[c.face];
  var out: Splat;
  out.corner = vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u)) * 2.0 - 1.0;
  let site = splat_site(vertex, splat);
  out.slant = size.x / (sqrt(2.0) * size.y);
  out.face = c.face;
  let fog = 1.0 - exp(-c.distance / globals.fog_distance);
  let tinted = unpack4x8unorm(c.tint).rgb * (0.3 + 0.7 * dot(paint.rgb, vec3f(0.3, 0.59, 0.11)));
  out.colour = vec4f(mix(select(paint.rgb, tinted, c.tint != 0u), globals.horizon.rgb, fog), paint.a);
  let at = site + vec3f(out.corner * size, 0.0);
  let p = transpose(face.view) * at;
  out.position = vec4f(p.xy * globals.focal / (globals.viewport / 2.0), depth_of(c.level, length(site)) * p.z, p.z);
  return out;
}

struct Tile {
  @builtin(position) position: vec4f,
  // Where on the plane of its nearest column, over that plane's distance: the ray's tangents in face coordinates.
  @location(0) ray: vec2f,
  @location(1) @interpolate(flat) tile: u32,
}

// A tile (its instance): a quad on the plane of its nearest column,
// holding its box's outline there and at its farthest, which march cuts to
// what its columns show.
@vertex
fn tile_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) tile: u32) -> Tile {
  let t = tiles[tile];
  let c = chunks[t.slot_count >> 8u];
  let near = max(c.distance + t.near, 1e-3);
  // The far end's outline, on the near end's plane: nearer the axis, smaller.
  let k = near / max(c.distance + t.far, near);
  let lo = c.origin + t.min;
  let hi = c.origin + t.max;
  let at = vec3f(mix(min(lo, lo * k), max(hi, hi * k), vec2f(f32(vertex & 1u), f32(vertex >> 1u))), near);
  let p = transpose(globals.faces[c.face].view) * at;
  return Tile(vec4f(p.xy * globals.focal / (globals.viewport / 2.0), depth_of(c.level, near) * p.z, p.z), at.xy / at.z, tile);
}

// What a pixel's ray meets first of a tile's columns (march): how far
// along the axis, the site of the column's voxel there, its colour, and
// how many voxels that is past the column's front; z 0, none.
struct Hit {
  z: f32,
  site: vec3f,
  paint: vec4f,
  past: f32,
}

// The index of a tile's column at (a, b) from its first, in its rows,
// a row's columns 2 apart in a; TILE · TILE, none.
fn column_at(d: vec2i) -> u32 {
  let s = (d.x - (d.y & 1)) / 2;
  if any(vec2i(s, d.y) < vec2i(0)) || any(vec2i(s, d.y) >= vec2i(TILE)) {
    return u32(TILE * TILE);
  }
  return u32(d.y * TILE + s);
}

const LAYER = 0.70710678;

// The column whose cell holds q, in units of the grid's `spacing` from a
// column's centre: the nearest centre, of the rows about it.
fn nearest(q: vec2f, spacing: vec2f) -> vec2i {
  var cell = vec2i(0);
  var best = 1e30;
  for (var row = floor(q.y); row <= floor(q.y) + 1.0; row += 1.0) {
    let a = 2.0 * round((q.x - row) / 2.0) + row;
    let d = (q - vec2f(a, row)) * spacing;
    if dot(d, d) < best {
      best = dot(d, d);
      cell = vec2i(i32(a), i32(row));
    }
  }
  return cell;
}

// The point of a chunk's picture drawn flat at q from its origin, relative
// to the camera: on its plane, moved across it as it is enlarged
// (compositor.rs's Flat).
fn flat_at(c: Chunk, q: vec2f) -> vec3f {
  return vec3f(c.origin + c.scale * q + c.offset, c.distance + c.plane.x + dot(c.plane.yz, q));
}

// Drawn flat, depths are those of points on the pictures' planes, which
// the GPU interpolates across a quad, so that the pixels draw what is
// nearest without working out their own (a fragment shader's depth would
// keep this GPU from testing it before the shader): as depth_of, level
// L's in [2^(L - 64), 2^(L - 63)), but of w, how far ahead along the view,
// affine in 1 / w, from w = NEAR · 2^L: far nearer the eye than a cell of
// L comes but at level 0, where it is near enough not to matter.
const NEAR = 0.0625;
fn flat_depth(level: f32, w: f32) -> f32 {
  return exp2(level - 64.0) * max(2.0 - NEAR * exp2(level) / w, 0.0);
}

// A run of a picture drawn flat (its instance: compositor.rs's flatten),
// of the cluster whose index · 4 is its first vertex: its columns'
// bricks, a quad on the picture's plane; how far into the run, and where
// its colours are, how many, and its chunk's slot. Only the face's own
// rays (owns): |x| + √2 |y| ≤ z in its coordinates (view.rs's in_face),
// so the quad is cut by four planes through the eye.
struct Run {
  @builtin(position) position: vec4f,
  @builtin(clip_distances) owned: array<f32, 4>,
  @location(0) along: f32,
  @location(1) @interpolate(flat) colours: vec3u,
}

// A run's pixel: Run less what only cuts its quad.
struct RunPixel {
  @location(0) along: f32,
  @location(1) @interpolate(flat) colours: vec3u,
}

@vertex
fn flat_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Run {
  let t = tiles[vertex >> 2u];
  let slot = t.slot_count >> 8u;
  let c = chunks[slot];
  let run = columns[t.first + instance];
  let row = f32(run.x & 0x3ffu);
  let first = f32((run.x >> 10u) & 0x7ffu);
  let length = run.x >> 21u;
  let corner = vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
  // Rows alternate from the first, even: an odd row's bricks half a brick on.
  let brick = vec2f(2.0 * (first + corner.x * f32(length)) + row % 2.0 - 1.0, row + corner.y - 0.5);
  let q = t.base + brick * vec2f(LAYER, 1.0) * c.half_layer * 2.0 / LAYER;
  let at = flat_at(c, q);
  let p = transpose(globals.faces[c.face].view) * at;
  var out: Run;
  out.position = vec4f(p.xy * globals.focal / (globals.viewport / 2.0), flat_depth(c.level, p.z) * p.z, p.z);
  let y = sqrt(2.0) * at.y;
  out.owned = array(at.z - at.x - y, at.z - at.x + y, at.z + at.x - y, at.z + at.x + y);
  out.along = corner.x * f32(length);
  out.colours = vec3u(t.first * 2u + run.y, length, slot);
  return out;
}

// The colour of the run's column there.
@fragment
fn flat_fragment(in: RunPixel) -> @location(0) vec4f {
  let k = in.colours.x + min(u32(max(in.along, 0.0)), in.colours.y - 1u);
  let colour = columns[k / 2u][k % 2u];
  return vec4f(shade(chunks[in.colours.z], unpack4x8unorm(colour)), 1.0);
}

// The first of a tile's columns the pixel's ray meets. A column is a
// pillar along the axis over its cell, its centre's Voronoi cell among the
// columns' centres (a LAYER, b), in units of the grid: |u| ≤ LAYER and
// LAYER |u| + |v| ≤ 3/4. Unlike voxels' outlines, which interlock, these
// share out the plane, so the ray crosses them one after another, from
// where it enters the tile's box to where it leaves, and meets a pillar in
// the cell it crosses it in. None if its face does not draw the ray (owns).
fn march(in: Tile) -> Hit {
  let none = Hit(0.0, vec3f(0.0), vec4f(0.0), 0.0);
  let ray = in.ray;
  let t = tiles[in.tile];
  let c = chunks[t.slot_count >> 8u];
  if !owns(c.face, in.position) {
    return none;
  }
  // The grid's spacing, across and up: a half layer is LAYER / 2 of it.
  let unit = c.half_layer * 2.0 / LAYER;
  let spacing = vec2f(LAYER, 1.0) * unit;
  let base = c.origin + t.base;
  // Where along the axis the ray is in the tile's box.
  let lo = c.origin + t.min;
  let hi = c.origin + t.max;
  var z = vec2f(max(c.distance + t.near, 1e-3), c.distance + t.far);
  for (var i = 0; i < 2; i++) {
    if ray[i] != 0.0 {
      let ends = vec2f(lo[i], hi[i]) / ray[i];
      z = vec2f(max(z.x, min(ends.x, ends.y)), min(z.y, max(ends.x, ends.y)));
    } else if lo[i] > 0.0 || hi[i] < 0.0 {
      return none;
    }
  }
  if z.x > z.y {
    return none;
  }
  // The cell it starts in.
  var cell = nearest((z.x * ray - base) / spacing, spacing);
  // The sides it may leave a cell past, n · (z · ray − centre) = d: those
  // it heads towards (n · ray > 0), at most one of each opposite two.
  let sides = select(vec3f(-1.0), vec3f(1.0), vec3f(ray.x, LAYER * ray.x + ray.y, LAYER * ray.x - ray.y) >= vec3f(0.0));
  let n0 = vec2f(sides.x, 0.0);
  let n1 = sides.y * vec2f(LAYER, 1.0);
  let n2 = sides.z * vec2f(LAYER, -1.0);
  let towards = vec3f(dot(n0, ray), dot(n1, ray), dot(n2, ray));
  let heads = towards > vec3f(0.0);
  let per = 1.0 / select(vec3f(1.0), towards, heads);
  let d = vec3f(LAYER, 0.75, 0.75) * unit;
  let steps = array(vec2i(2 * i32(sides.x), 0), vec2i(i32(sides.y)), vec2i(1, -1) * i32(sides.z));
  var enter = z.x;
  for (var crossed = 0; crossed < 4 * TILE; crossed++) {
    let centre = base + vec2f(cell) * spacing;
    let exits = select(vec3f(z.y), (d + vec3f(dot(n0, centre), dot(n1, centre), dot(n2, centre))) * per, heads);
    let leave = min(z.y, min(exits.x, min(exits.y, exits.z)));
    let next = cell + select(select(steps[2], steps[1], leave == exits.y), steps[0], leave == exits.x);
    let k = column_at(cell);
    if k < u32(TILE * TILE) {
      let column = columns[t.first + k];
      let paint = unpack4x8unorm(column.y);
      let depth = bitcast<i32>(column.x << 16u) >> 16u;
      let span = c.distance + vec2f(f32(depth), f32(depth + (bitcast<i32>(column.x) >> 16u))) * c.half_layer;
      let at = max(enter, span.x);
      if column.y != 0u && at <= min(leave, span.y) {
        // Of the column's voxels, every two layers from its front.
        let apart = 4.0 * c.half_layer;
        let past = round((at - span.x) / apart);
        return Hit(at, vec3f(centre, span.x + past * apart), paint, past);
      }
    }
    if leave >= z.y {
      break;
    }
    enter = leave;
    cell = next;
  }
  return none;
}

// Its colour as the chunk paints it (Chunk's tint) and fogged.
fn shade(c: Chunk, paint: vec4f) -> vec3f {
  let fog = 1.0 - exp(-c.distance / globals.fog_distance);
  let tinted = unpack4x8unorm(c.tint).rgb * (0.3 + 0.7 * dot(paint.rgb, vec3f(0.3, 0.59, 0.11)));
  return mix(select(paint.rgb, tinted, c.tint != 0u), globals.horizon.rgb, fog);
}

struct Drawn {
  @location(0) colour: vec4f,
  @builtin(frag_depth) depth: f32,
}

// A hit, at the depth of the site of the column's voxel it is in, so that
// where a pillar stands for whole voxels, the one a ray meets first is in
// front (depth_of); no nearer than its tile's quad, at its nearest
// column's front, so that what is drawn already hides the tile's pixels
// before they march (early_depth_test). A pillar running on past its cell
// meets voxels another cell's columns start at, at the same site: there
// the one whose front is nearer the voxel, in a colour more its own, is a
// hair nearer, so that the order they are drawn in mostly does not matter.
fn shown(in: Tile, hit: Hit) -> Drawn {
  let t = tiles[in.tile];
  let c = chunks[t.slot_count >> 8u];
  let hair = 0.01 * c.half_layer * hit.past;
  let distance = max(length(hit.site) + hair, max(c.distance + t.near, 1e-3));
  return Drawn(vec4f(shade(c, hit.paint), 1.0), depth_of(c.level, distance));
}

// A whole voxel, as a splat: if the pixel is in its hexagon, and its face
// draws its ray.
@fragment
fn splat_fragment(in: Splat) -> @location(0) vec4f {
  if abs(in.corner.x) * in.slant + abs(in.corner.y) >= 1.0 || !owns(in.face, in.position) {
    discard;
  }
  return vec4f(in.colour.rgb, 1.0);
}

// A whole voxel, of a relief.
@fragment @early_depth_test(greater_equal)
fn tile_fragment(in: Tile) -> Drawn {
  let hit = march(in);
  if hit.z == 0.0 {
    discard;
  }
  return shown(in, hit);
}

// Whether the pixel's ray is the face's, for one face only, so that no
// pixel is left to none or drawn by two: its face is the neighbour
// direction (±1, ±1, 0) along its two largest components, the smallest
// left out (seams_fragment), ties to the last.
fn owns(face: u32, position: vec4f) -> bool {
  let axis = globals.faces[face].axis.xyz;
  let ray = ray_of(globals.sky_view, position);
  let a = abs(ray);
  let least = select(select(2u, 1u, a.y < a.z), 0u, a.x < min(a.y, a.z));
  return axis[least] == 0.0 && dot(axis, sign(ray)) == 2.0;
}

// A layer's depth along its face's axis, 1/√2, over a voxel's mean chord,
// 4/(3√2) (worldgen's CHORD), both in units of its grid.
const LAYER_OVER_CHORD = 0.75;

// A see-through one, in front of the whole ones. A layer's sites are on a
// grid twice their outlines' half width and height apart (half the plane
// their outlines, the next layer's the rest), so a tent of twice that
// over each sums to one across the layer: the layer a field, each of
// its pixels as much of each site as it is near, the same along a ray
// across seams and from any angle. A site's alpha is what it stops of
// rays crossing it, along its voxel's mean chord (bake's alpha): the ray's
// own path through the layer stops 1 − (1 − alpha)^(path / chord) of it.
// Drawn nearest first, each under those before it, which let through
// `a` of the ray, its colour as much as it stops, and the ray on through
// it (the pipeline's blend). A face's rays meet its layers in order of
// depth along its axis, and cells as their sites are near the eye
// (Compositor::prepare), up to tents of neighbouring cells overlapping,
// whose order then is the cells', a small error where alpha is small. Each
// pixel is one face's only (owns), so seams count once.
@fragment
fn veil_fragment(in: Splat) -> @location(0) vec4f {
  if !owns(in.face, in.position) {
    discard;
  }
  let ray = ray_of(globals.faces[in.face].view, in.position);
  let share = (1.0 - abs(in.corner.x)) * (1.0 - abs(in.corner.y));
  let path = LAYER_OVER_CHORD * length(ray) / ray.z;
  let a = 1.0 - pow(1.0 - in.colour.a, share * path);
  return vec4f(in.colour.rgb * a, a);
}

// Over the picture: their colour, and as much of it as they let through.
@fragment
fn composite_fragment(in: Sky) -> @location(0) vec4f {
  return textureLoad(veil, vec2i(in.position.xy), 0);
}

// |v| for the v in [lo, hi] nearest to 0.
fn closest_to_zero(lo: f32, hi: f32) -> f32 {
  return select(select(0.0, -hi, hi < 0.0), lo, lo > 0.0);
}

// Where a cluster may draw on screen, in pixels (left, top, right, bottom),
// within its face's rays, a hair wider (owns); empty if it draws
// nowhere.
fn screen_box(cluster: Cluster) -> vec4f {
  let c = chunks[cluster.slot_count >> 8u];
  // The rays through its box, as tangents, at most its corners', near and
  // far; a splat's widened by its slant across (splat_vertex), a
  // see-through one's as much, to hold where its tent is drawn.
  let z = c.distance + vec2f(cluster.near, cluster.far);
  if z.x <= 0.0 {
    return vec4f(vec2f(0.0), globals.resolution);
  }
  var lo = c.origin + cluster.min;
  var hi = c.origin + cluster.max;
  let slant = max(max(abs(lo.x) / z.x, abs(hi.x) / z.x), max(abs(lo.x) / z.y, abs(hi.x) / z.y));
  lo.x -= cluster.wide * slant;
  hi.x += cluster.wide * slant;
  lo = min(lo / z.x, lo / z.y);
  hi = max(hi / z.x, hi / z.y);
  if closest_to_zero(lo.x, hi.x) + sqrt(2.0) * closest_to_zero(lo.y, hi.y) > 1.001 {
    return vec4f(0.0);
  }
  let view = transpose(globals.faces[c.face].view);
  var b = vec4f(globals.resolution, vec2f(0.0));
  for (var k = 0u; k < 4u; k++) {
    let p = view * vec3f(select(lo.x, hi.x, (k & 1u) != 0u), select(lo.y, hi.y, k >= 2u), 1.0);
    // A corner behind the camera: the box goes off the screen, so take all of it.
    if p.z <= 0.0 {
      return vec4f(vec2f(0.0), globals.resolution);
    }
    let s = (p.xy * globals.focal / (globals.viewport / 2.0) / p.z * vec2f(0.5, -0.5) + 0.5) * globals.resolution;
    b = vec4f(min(b.xy, s), max(b.zw, s));
  }
  return clamp(b, vec4f(0.0), globals.resolution.xyxy);
}

fn shows(b: vec4f) -> bool {
  return all(b.xy < b.zw);
}

// Whether what is drawn already is nearer than `depth` all over box `b`:
// the farthest depth over it, from the level of the pyramid at which it
// spans at most two texels each way.
fn hidden(b: vec4f, depth: f32) -> bool {
  let levels = i32(textureNumLevels(farthest));
  let size = max(b.z - b.x, b.w - b.y);
  let level = clamp(i32(ceil(log2(max(size, 1.0)))) - 1, 0, levels - 1);
  let texel = f32(2 << u32(level));
  // Each level halves the last, down to a texel (Targets::new). Asked for
  // one level per thread, Mesa's lavapipe gave some threads another level's size.
  let last = max(vec2i(textureDimensions(farthest)) >> vec2u(u32(level)), vec2i(1)) - 1;
  let start = min(vec2i(floor(b.xy / texel)), last);
  let end = min(vec2i(ceil(b.zw / texel)) - 1, last);
  var far = 0.0;
  for (var y = start.y; y <= end.y; y++) {
    for (var x = start.x; x <= end.x; x++) {
      far = max(far, textureLoad(farthest, vec2i(x, y), level).r);
    }
  }
  // Rasterising rounds the depth a few ulps either way: ties show.
  return depth * (1.0 - exp2(-18.0)) > far;
}

// The least depth a cluster's columns or splats may be drawn at: of its
// nearest column's (or splat's) distance, no less than how far ahead it
// is; drawn flat, of the least w in its box.
fn nearest_depth(cluster: Cluster) -> f32 {
  let c = chunks[cluster.slot_count >> 8u];
  if globals.whole == FLAT {
    let ahead = globals.faces[c.face].view[2];
    let lo = ahead * vec3f(c.origin + cluster.min, c.distance + cluster.near);
    let hi = ahead * vec3f(c.origin + cluster.max, c.distance + cluster.far);
    let w = dot(min(lo, hi), vec3f(1.0));
    return select(0.0, flat_depth(c.level, w), w > 0.0);
  }
  return depth_of(c.level, max(c.distance + cluster.near, 0.0));
}

// Off, every cluster in view is drawn: for tests, as what culling must not change.
override culls = true;

// Whether to draw a cluster in a phase; in the second, whether it shows is
// kept for the next frame.
fn draws_cluster(at: u32, cluster: Cluster, phase: u32) -> bool {
  let b = screen_box(cluster);
  if phase == 0u {
    return cluster.visible != 0u && shows(b);
  }
  let visible = shows(b) && !(culls && hidden(b, nearest_depth(cluster)));
  clusters[at].visible = u32(visible);
  // The first phase drew those visible before and in view.
  return visible && cluster.visible == 0u;
}

// The frame's `k`-th cluster, of a chunk of `chunks` after the first item,
// which holds how many chunks and clusters there are: each chunk's
// [first, first + count) of `clusters`, and how many of the frame's come
// before (so, which chunk's: the last whose come no later); none past the
// last.
fn candidate(k: u32) -> i32 {
  if k >= candidates[0].y {
    return -1;
  }
  var lo = 1u;
  var hi = candidates[0].x + 1u;
  while hi - lo > 1u {
    let mid = (lo + hi) / 2u;
    if candidates[mid].z <= k {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  return i32(candidates[lo].x + k - candidates[lo].z);
}

// The see-through ones, as `candidate`.
fn veiled_candidate(k: u32) -> i32 {
  if k >= veiled[0].y {
    return -1;
  }
  var lo = 1u;
  var hi = veiled[0].x + 1u;
  while hi - lo > 1u {
    let mid = (lo + hi) / 2u;
    if veiled[mid].z <= k {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  return i32(veiled[lo].x + k - veiled[lo].z);
}

// A thread per cluster (a tile, or whole splats) of the frame. Chunks come
// near to far and their tiles nearest first (see tiles and cluster in
// compositor.rs), and draws are listed about in that order, so that
// nearer voxels are mostly drawn first and hide what is behind them early.
fn cull(k: u32, phase: u32) {
  let at = candidate(k);
  if at < 0 {
    return;
  }
  let cluster = clusters[at];
  if draws_cluster(u32(at), cluster, phase) {
    var draw = DrawArgs(4u, 1u, 0u, u32(at));
    if globals.whole == 1.0 {
      draw = DrawArgs(4u, cluster.slot_count & 0xffu, (cluster.slot_count >> 8u) * 4u, cluster.first);
    } else if globals.whole == FLAT {
      draw = DrawArgs(4u, cluster.slot_count & 0xffu, u32(at) * 4u, 0u);
    }
    draws[atomicAdd(&drawn, 1u)] = draw;
  }
}

// The thread's place among those of its dispatch (compositor.rs's groups).
fn thread_of(id: vec3u, n: vec3u, thread: u32) -> u32 {
  return (id.y * n.x + id.x) * 64u + thread;
}

@compute @workgroup_size(64)
fn cull_drawn(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  cull(thread_of(id, n, thread), 0u);
}

@compute @workgroup_size(64)
fn cull_rest(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  cull(thread_of(id, n, thread), 1u);
}

// A draw for each see-through cluster, in their order, of none of its
// splats where it does not show: in front of what the first phase drew.
@compute @workgroup_size(64)
fn cull_see_through(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  let k = thread_of(id, n, thread);
  let at = veiled_candidate(k);
  if at < 0 {
    return;
  }
  let cluster = clusters[at];
  let b = screen_box(cluster);
  let shows = shows(b) && !(culls && hidden(b, nearest_depth(cluster)));
  veil_draws[k] = DrawArgs(4u, select(0u, cluster.slot_count & 0xffu, shows), (cluster.slot_count >> 8u) * 4u, cluster.first);
}

// A texel of level 0 is the farthest of 2 × 2 pixels, one of a level the
// farthest of 2 × 2 texels of the last; past the edge, the edge's.
@compute @workgroup_size(8, 8)
fn reduce_depth(@builtin(global_invocation_id) id: vec3u) {
  if any(id.xy >= textureDimensions(level_out)) {
    return;
  }
  let last = vec2i(textureDimensions(depth_in)) - 1;
  let p = vec2i(id.xy) * 2;
  let far = max(
    max(textureLoad(depth_in, min(p, last), 0), textureLoad(depth_in, min(p + vec2i(1, 0), last), 0)),
    max(textureLoad(depth_in, min(p + vec2i(0, 1), last), 0), textureLoad(depth_in, min(p + 1, last), 0)),
  );
  textureStore(level_out, id.xy, vec4f(far));
}

@compute @workgroup_size(8, 8)
fn reduce(@builtin(global_invocation_id) id: vec3u) {
  if any(id.xy >= textureDimensions(level_out)) {
    return;
  }
  let last = vec2i(textureDimensions(level_in)) - 1;
  let p = vec2i(id.xy) * 2;
  let far = max(
    max(textureLoad(level_in, min(p, last), 0).r, textureLoad(level_in, min(p + vec2i(1, 0), last), 0).r),
    max(textureLoad(level_in, min(p + vec2i(0, 1), last), 0).r, textureLoad(level_in, min(p + 1, last), 0).r),
  );
  textureStore(level_out, id.xy, vec4f(far));
}

struct Sky {
  @builtin(position) position: vec4f,
}

@vertex
fn sky_vertex(@builtin(vertex_index) vertex: u32) -> Sky {
  let corner = vec2f(f32(vertex & 1u), f32(vertex >> 1u)) * 2.0 - 1.0;
  return Sky(vec4f(corner, 1.0, 1.0));
}

@fragment
fn sky_fragment(in: Sky) -> @location(0) vec4f {
  let ray = ray_of(globals.sky_view, in.position);
  let elevation = ray.y / length(ray.xz);
  let rgb = mix(globals.horizon.rgb, globals.sky_top.rgb, clamp(elevation / 0.625, 0.0, 1.0));
  return vec4f(mix(rgb, globals.horizon.rgb, globals.sky_fog), 1.0);
}

// The seams between faces, over the picture. A ray's face is the neighbour
// direction (±1, ±1, 0) along its two largest components, so faces meet
// where the second and third largest are equal in size.
@fragment
fn seams_fragment(in: Sky) -> @location(0) vec4f {
  let a = abs(ray_of(globals.sky_view, in.position));
  let second = max(min(a.x, a.y), min(max(a.x, a.y), a.z));
  let third = min(min(a.x, a.y), a.z);
  // Pixels to the seam, near it; the seam 1.5 points wide.
  let pixels = (second - third) / max(fwidth(second - third), 1e-9);
  let half = 0.75 * globals.resolution.x / globals.viewport.x;
  return vec4f(1.0, 0x2f / 255.0, 0xd0 / 255.0, 1.0 - smoothstep(half - 0.5, half + 0.5, pixels));
}

struct Shown {
  @builtin(position) position: vec4f,
  @location(0) at: vec2f,
}

// The frame onto the view's rectangle, its viewport.
@vertex
fn show_vertex(@builtin(vertex_index) vertex: u32) -> Shown {
  let corner = vec2f(f32(vertex & 1u), f32(vertex >> 1u));
  return Shown(vec4f(corner * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), corner);
}

@fragment
fn show_fragment(in: Shown) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(picture));
  return textureLoad(picture, vec2i(min(in.at * size, size - 1.0)), 0);
}

