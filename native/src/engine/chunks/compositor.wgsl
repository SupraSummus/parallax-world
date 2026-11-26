// The chunks' compositor's shaders (compositor.rs), for cells' pictures
// of whole voxels (bake's cell_relief, compositor.rs's flatten), and
// pictures of their medium, drawing only the clusters of them that may
// show: each cluster names its chunk's slot in `chunks`, which gives its
// face and plane relative to the camera, as `place` makes it each frame of
// the chunk's record. Then cull_drawn lists draws of the runs of pictures
// that showed last frame, and flat_vertex draws them; their depth is
// reduced to a pyramid of farthest depths (reduce_depth, reduce), and
// cull_rest tests every cluster against it: those that showed are kept for the next frame,
// and those newly showing drawn now. A phase's draws are listed in the
// clusters' order, those it leaves out left out (list, scan_draws,
// pack_draws), so that where depths tie, the same shows every time.
// cull_rest also counts the pictures of medium in front of them by their
// place along the rays, which scan_veils and order_veils list far to
// near, to be laid one over another on the picture (veil_fragment).

// A face's own rays, for pictures drawn flat and of medium (flat_vertex, veil_vertex);
// what a picture of medium lets through of each channel (veil_fragment).
enable clip_distances;
enable dual_source_blending;

struct FaceView {
  // Columns: the camera's right, up and forward axes, in the face's coordinates.
  view: mat3x3f,
  // Its lattice vector F (bake's Face), and its layers' spacing.
  axis: vec4f,
  // The camera, in the face's coordinates, and the sum of F's components.
  eye: vec4f,
  // Its rays' tangents across and up at most, k (1 along an axis, 2 along
  // an edge), and the sum of F's positive components.
  slant: vec4f,
}

struct Globals {
  viewport: vec2f, // CSS pixels (points)
  resolution: vec2f, // device pixels
  focal: f32,
  fog_distance: f32,
  pad: f32,
  sky_fog: f32,
  sky_top: vec4f,
  horizon: vec4f,
  sky_view: mat3x3f, // the camera's axes in world coordinates
  faces: array<FaceView, 18>,
  // Which cells each level claims (claims), and which the medium's
  // (medium_claims), by their Lods (Lod::for_medium): each its Q, the
  // render distance, its coarsest level and its finest.
  lod: vec4i,
  medium: vec4i,
}

// A chunk where it is, wherever the camera (compositor.rs's GpuRecord):
// in its face's coordinates, where its column (0, 0) is, and its plane's
// depth; its face; its cell's level, its grid's, and twice its middle's c
// in its grid's lattice coordinates; its tint; the box its columns fill from its
// origin, and its picture's plane past its own (z0, gx, gy) and how far its
// edge pillars reach from it; its clusters of whole voxels, [first, end) of
// `clusters`, none if its slot is free; a half layer of its grid; the end
// of its clusters of pictures of medium, which follow its whole voxels'.
struct Record {
  origin: vec2f,
  depth: f32,
  face: u32,
  level: i32,
  atoms: i32,
  centre: i32,
  tint: u32,
  min: vec2f,
  max: vec2f,
  plane: vec4f,
  whole: vec2u,
  half_layer: f32,
  veils: u32,
}

// A chunk as the frame draws it, from the camera (place).
struct Chunk {
  origin: vec2f, // its column (0, 0), relative to the camera, face coordinates
  distance: f32, // of the centre plane, along the face's axis
  face: u32,
  level: f32, // see flat_depth
  tint: u32, // RGBA to paint its voxels in, shaded as they are; 0 for their own
  half_layer: f32, // a half layer of its grid: the unit of its columns' depths
  clip: f32, // its medium from past this layer (see veil_fragment), in half layers past its plane
  // Its picture (compositor.rs's Flat): its picture's q from its origin at
  // scale · q + offset, its plane past its own, z0 + gx x + gy y at q =
  // (x, y); and w, 1 if its level claims its cell from where the eye is,
  // else 0: not drawn.
  scale: vec2f,
  offset: vec2f,
  plane: vec4f,
  whole: vec2u, // its record's
  veils: u32, // its record's
  reach: f32, // and to this one
}

// `count` (low byte of `slot_count`) runs of a picture (GpuCluster) from
// `first`, its first column's centre `base`; or a picture of medium at
// `first`, its slab's first layer `layer` and how
// deep its halves are, `half`, in half layers (see veil_fragment). Its
// chunk's slot (`slot_count` above the low byte), the box of their outlines along the
// axis relative to the chunk's origin, whether any showed in the last
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
  layer: f32,
  half: f32,
  pad: u32, // as long as compositor.rs's GpuCluster
}

// A hidden cell (compositor.rs's GpuProbe), which find_probes asks whether
// it shows: its box in its face's coordinates, across and up from `min`
// to `max`, and along the axis, `depth` near and far; its face, its
// cell's level and its layer of cells, K (claims), and its id.
struct Probe {
  min: vec2f,
  max: vec2f,
  depth: vec2f,
  face: u32,
  level: i32,
  layer: i32,
  id: u32,
}

// A draw: a cluster's runs, instances of flat_vertex or veil_vertex.
struct DrawArgs {
  vertices: u32,
  instances: u32,
  first_vertex: u32,
  first_instance: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> chunks: array<Chunk>;
// Drawing: the clusters, and their pictures' items (GpuColumn).
@group(0) @binding(2) var<storage, read> tiles: array<Cluster>;
@group(0) @binding(3) var<storage, read> columns: array<vec2u>;
// Culling: all the clusters, some left where chunks were (cull), and the
// draws of those drawn, in their order; listed by workgroup first, how
// many each (list), and how many in all (scan_draws, pack_draws).
@group(0) @binding(4) var<storage, read_write> clusters: array<Cluster>;
@group(0) @binding(6) var<storage, read_write> draws: array<DrawArgs>;
@group(0) @binding(24) var<storage, read_write> listed: array<DrawArgs>;
@group(0) @binding(25) var<storage, read_write> blocks: array<u32>;
@group(0) @binding(8) var farthest: texture_2d<f32>;
// The halves of pictures of medium to draw, far to near, and how many; in two
// parts, how many at each place (veil_place), then where in veil_draws
// the next of each goes.
@group(0) @binding(9) var<storage, read_write> veils_drawn: atomic<u32>;
@group(0) @binding(16) var<storage, read_write> veil_draws: array<DrawArgs>;
@group(0) @binding(22) var<storage, read_write> veil_places: array<atomic<u32>>;
// Hidden cells' probes (find_probes), and those that show: how many, how
// many probes there are, then the ids of those that show.
@group(0) @binding(26) var<storage, read> probes: array<Probe>;
@group(0) @binding(23) var<storage, read_write> found: array<atomic<u32>>;
// The chunks' records, by slot, and the chunks place makes of them.
@group(0) @binding(17) var<storage, read> records: array<Record>;
@group(0) @binding(18) var<storage, read_write> made: array<Chunk>;
// Writing into a buffer (compositor.rs's Arena::stage): the words written,
// one after another, and their runs, each [where its first goes, in words,
// its first in `written`, how many], after how many runs and words in all.
@group(0) @binding(19) var<storage, read_write> into: array<u32>;
@group(0) @binding(20) var<storage, read> written: array<u32>;
@group(0) @binding(21) var<storage, read> runs: array<vec4u>;
// The pyramid: level 0 from the depth buffer, each next from the last.
@group(0) @binding(10) var depth_in: texture_depth_2d;
@group(0) @binding(11) var level_in: texture_2d<f32>;
@group(0) @binding(12) var level_out: texture_storage_2d<r32float, write>;
// Showing the frame.
@group(0) @binding(13) var picture: texture_2d<f32>;
// The picture, finer than its own format, with the pictures of medium laid on it (veil_fragment).
@group(0) @binding(14) var veil: texture_2d<f32>;

fn ray_of(view: mat3x3f, position: vec4f) -> vec3f {
  let s = position.xy / globals.resolution - 0.5;
  return view * vec3f(vec2f(s.x, -s.y) * globals.viewport / globals.focal, 1.0);
}

// The point of a chunk's picture drawn flat at q from its origin, relative
// to the camera: on its plane, moved across it as it is enlarged
// (compositor.rs's Flat).
fn flat_at(c: Chunk, q: vec2f) -> vec3f {
  return vec3f(c.origin + c.scale * q + c.offset, c.distance + c.plane.x + dot(c.plane.yz, q));
}

// Depths are those of points on the pictures' planes, which the GPU
// interpolates across a quad, so that the pixels draw what is nearest
// without working out their own (a fragment shader's depth would keep
// the GPU from testing it before the shader): finer levels in front
// (lod.rs), level L's in [2^(L - 64), 2^(L - 63)), of w, how far ahead along the view,
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
// rays (owns, bake's Face::holds), so the quad is cut by planes through
// the eye: overlapping past a seam, faces' fragments tie in depth by
// chance, and the one drawn last would show, in no set order.
struct Run {
  @builtin(position) position: vec4f,
  @builtin(clip_distances) owned: array<f32, 6>,
  @location(0) along: f32,
  @location(1) @interpolate(flat) colours: vec3u,
}

// A run's pixel: Run less what only cuts its quad.
struct RunPixel {
  @location(0) along: f32,
  @location(1) @interpolate(flat) colours: vec3u,
}

// Where corner `vertex` of a run (packed: compositor.rs's packed) of a
// picture is on it, in bricks across and rows up from its first place's
// centre.
fn run_corner(run: u32, vertex: u32) -> vec2f {
  let row = f32(run & 0x3ffu);
  let first = f32((run >> 10u) & 0x7ffu);
  let corner = vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
  return vec2f(first + corner.x * f32(run >> 21u) - 0.5, row + corner.y - 0.5);
}

// A brick of a chunk's picture, a column of its grid: a layer wide, a site high.
fn brick_of(c: Chunk) -> vec2f {
  let layer = globals.faces[c.face].axis.w;
  return vec2f(layer, 1.0) * c.half_layer * 2.0 / layer;
}

// That corner from its chunk's origin, the cluster `t` its first place's centre at its base.
fn run_at(t: Cluster, c: Chunk, corner: vec2f) -> vec2f {
  return t.base + corner * brick_of(c);
}

// A quad's corner at `at` from the eye in its chunk's face's coordinates, drawn at `depth`.
fn quad_position(c: Chunk, at: vec3f, depth: f32) -> vec4f {
  let p = transpose(globals.faces[c.face].view) * at;
  return vec4f(p.xy * globals.focal / (globals.viewport / 2.0), depth * p.z, p.z);
}

// How far `at` is within face `face`'s own rays, by each of the planes
// cutting them (bake's Face::planes): along an axis, |x| and |y| ≤ TAN z;
// along an edge, |x| ≤ TAN z and |x| + √2 |y| ≤ z.
const TAN = 0.41421356237309515;
fn in_face(face: u32, at: vec3f) -> array<f32, 6> {
  let t = TAN * at.z;
  if globals.faces[face].slant.z == 1.0 {
    return array(t - at.x, t + at.x, t - at.y, t + at.y, 1.0, 1.0);
  }
  let y = sqrt(2.0) * at.y;
  return array(t - at.x, t + at.x, at.z - at.x - y, at.z - at.x + y, at.z + at.x - y, at.z + at.x + y);
}

@vertex
fn flat_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Run {
  let t = tiles[vertex >> 2u];
  let slot = t.slot_count >> 8u;
  let c = chunks[slot];
  let run = columns[t.first + instance];
  let at = flat_at(c, run_at(t, c, run_corner(run.x, vertex)));
  let w = dot(globals.faces[c.face].view[2], at);
  var out: Run;
  out.position = quad_position(c, at, flat_depth(c.level, w));
  out.owned = in_face(c.face, at);
  out.along = f32(vertex & 1u) * f32(run.x >> 21u);
  out.colours = vec3u(t.first * 2u + run.y, run.x >> 21u, slot);
  return out;
}

// The colour of the run's column there.
@fragment
fn flat_fragment(in: RunPixel) -> @location(0) vec4f {
  let k = in.colours.x + min(u32(max(in.along, 0.0)), in.colours.y - 1u);
  let colour = columns[k / 2u][k % 2u];
  return vec4f(shade(chunks[in.colours.z], unpack4x8unorm(colour)), 1.0);
}

// A half of a picture of medium (compositor.rs's veils), a quad on its slab's middle
// plane, as wide as its cluster's box but for the slants the face's rays
// do not reach where its chunk is. Its depth is the least anything past
// its slab's front may be drawn at (nearest_depth, as its culling has it),
// of the level whose whole voxels are drawn there (level_ahead): what a
// ray crosses of it lies before whatever the ray meets in the slab, so
// medium behind that is not drawn, and it shows before all within. Where on
// the picture, in bricks across and rows up from its first place's
// centre; where its picture is; which half of its slab it is
// (veil_fragment); how far aside a ray of slant 1 takes its
// columns there, toward the half; where it is from the eye; and how its chunk shades
// (Shading), worked out here: reading the globals for it in every
// fragment is slow.
struct Veil {
  @builtin(position) position: vec4f,
  @builtin(clip_distances) owned: array<f32, 6>,
  @location(0) place: vec2f,
  @location(1) @interpolate(flat) picture: u32,
  @location(2) @interpolate(flat) half: u32,
  @location(3) at: vec3f,
  @location(4) @interpolate(flat) grey: vec4f,
  @location(5) @interpolate(flat) weight: vec3f,
  @location(6) @interpolate(flat) aside: vec2f,
  @location(7) @interpolate(flat) along: f32,
}

struct VeilPixel {
  @location(0) place: vec2f,
  @location(1) @interpolate(flat) picture: u32,
  @location(2) @interpolate(flat) half: u32,
  @location(3) at: vec3f,
  @location(4) @interpolate(flat) grey: vec4f,
  @location(5) @interpolate(flat) weight: vec3f,
  @location(6) @interpolate(flat) aside: vec2f,
  @location(7) @interpolate(flat) along: f32,
}

@vertex
fn veil_vertex(@builtin(vertex_index) vertex: u32) -> Veil {
  let t = tiles[vertex >> 3u];
  let half = (vertex >> 2u) & 1u;
  let slot = t.slot_count >> 8u;
  let c = chunks[slot];
  // Its box holds the face's rays' slants: this chunk's, from its slab's front, at most.
  let front = c.distance + t.near;
  let far = max(abs(c.origin + t.min), abs(c.origin + t.max));
  let most = globals.faces[c.face].slant.xy;
  let slant = select(most, min(far / front, most), front > 0.0);
  let out_by = vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u)) * 2.0 - 1.0;
  let corner = select(t.min, t.max, out_by > vec2f(0.0)) - out_by * (most - slant) * t.half * c.half_layer;
  let at = vec3f(c.origin + corner, c.distance + t.far);
  let depth = nearest_depth(t, level_ahead(c.face, front));
  var out: Veil;
  out.position = quad_position(c, at, depth);
  out.owned = in_face(c.face, at);
  out.place = (corner - t.base) / brick_of(c);
  out.picture = t.first;
  out.half = half;
  // Half the slab's depth, in bricks across and up, nearer it toward the eye, farther away.
  let layer = globals.faces[c.face].axis.w;
  out.aside = select(-1.0, 1.0, half == 1u) * t.half * vec2f(0.5, layer / 2.0);
  out.at = at;
  let shading = shading_of(c);
  out.grey = shading.grey;
  out.weight = shading.weight;
  // A column's voxels' path along the axis each, k layers, in units of its grid, which their optical depths are of.
  out.along = globals.faces[c.face].slant.z * layer;
  return out;
}

// Optical depths of each channel from a word (compositor.rs's depth_bits).
fn depths(w: u32) -> vec3f {
  let q = vec3u(w, w >> 10u, w >> 20u) & vec3u(1023u);
  return select(exp2(vec3f(q) / DEPTH_STEPS - DEPTH_LEAST), vec3f(0.0), q == vec3u(0u));
}
const DEPTH_STEPS = 48.0;
const DEPTH_LEAST = 14.0;

// A half of a slab of a column of the picture of medium at `picture`, row
// q, column s (bake's Half): of each channel, its colour times its optical
// depth, and that depth; none outside the picture.
fn medium_at(picture: u32, q: i32, s: i32, half: u32) -> mat2x3f {
  let size = vec2i(columns[picture]);
  if q < 0 || s < 0 || q >= size.y || s >= size.x {
    return mat2x3f();
  }
  let column = columns[picture + 1u + 2u * u32(q * size.x + s) + half];
  let depth = depths(column.y);
  return mat2x3f(unpack4x8unorm(column.x).rgb * depth, depth);
}

// A row of a picture of medium at x, in bricks: between the two columns
// around x, each as much as x is near it.
fn medium_row(picture: u32, q: i32, x: f32, half: u32) -> mat2x3f {
  let s = i32(floor(x));
  let t = x - f32(s);
  return medium_at(picture, q, s, half) * (1.0 - t) + medium_at(picture, q, s + 1, half) * t;
}

// A picture of medium at `place`: its columns' as a field, each column's
// share of it a tent over twice its brick, so that they add up to one
// everywhere: across cells too, whose pictures, each its own columns'
// alone, lie over one another there.
fn medium_field(picture: u32, place: vec2f, half: u32) -> mat2x3f {
  let q = i32(floor(place.y));
  let t = place.y - f32(q);
  return medium_row(picture, q, place.x, half) * (1.0 - t) + medium_row(picture, q + 1, place.x, half) * t;
}

// Columns of a picture of medium a pixel takes along its ray through each
// half of a slab, at most.
const TAPS = 8;

// What a picture of medium sends of each channel, and lets through of
// what is behind it: the picture under them takes the one and that times
// the other (Pipelines's veil).
struct Veiled {
  @location(0) @blend_src(0) sent: vec4f,
  @location(0) @blend_src(1) through: vec4f,
}

// The medium of a half of a slab along a pixel's ray. A column's is taken as spread
// evenly through each half of its slab, as a pillar is solid
// through its depth: a ray slanting σ off the axis crosses the columns
// over σ times the half's depth, the nearer half's before where it
// crosses the slab's middle plane and the farther's after, and takes of
// each as much as it crosses, so a pixel takes the columns along that
// line, one a column but TAPS at most: a deeper slab's, more apart. So
// medium thin along the axis shows as much as the rays cross of it, not
// only where they cross the plane. A half is drawn if its level's (veil_place).
// A column's voxels are k layers apart, and its brick its share of the
// plane, so a ray along the axis crosses as much of each as k layers'
// depth, a slanted one as much more as its path through the slab is
// longer; a voxel's optical depth is of a path of one site (lattice's `over`).
// Each channel apart, as the voxels engine's rays take a medium: its colour
// shows as much as it stops of it, over what is behind, which it lets the
// rest of through; the halves come far to near (order_veils), every
// picture's farther half of a slab before any's nearer, and each
// is fogged by its distance, as whole voxels are. Of a
// slab's halves, those past its level's `clip`, where the next finer
// level's medium ends, and not past its `reach`, where its own does
// (place): each level's medium lies between two planes, which fall
// between halves (bake's slab_layers).
@fragment
fn veil_fragment(in: VeilPixel) -> Veiled {
  let aside = in.at.xy / in.at.z * in.aside;
  let taps = clamp(i32(ceil(max(abs(aside.x), abs(aside.y)))), 1, TAPS);
  var field = mat2x3f();
  for (var k = 0; k < taps; k++) {
    field += medium_field(in.picture, in.place + aside * (f32(k) + 0.5) / f32(taps), in.half);
  }
  let along = in.along * length(in.at) / in.at.z / f32(taps);
  let veil = veiled(field, along, Shading(in.grey, in.weight));
  return Veiled(vec4f(veil[0], 0.0), vec4f(veil[1], 1.0));
}

// What a half of a slab sends of each channel along `along` times its
// optical depth, its colour shaded, and what it lets through.
fn veiled(field: mat2x3f, along: f32, shading: Shading) -> mat2x3f {
  let through = exp(-field[1] * along);
  let colour = shaded(shading, vec4f(field[0] / max(field[1], vec3f(1e-30)), 1.0));
  return mat2x3f(colour * (1.0 - through), through);
}

// Its colour as the chunk paints it (Chunk's tint) and fogged.
fn shade(c: Chunk, paint: vec4f) -> vec3f {
  return shaded(shading_of(c), vec4f(paint.rgb, 1.0));
}

// How much a chunk's colours are fogged, by its distance.
fn fog_of(c: Chunk) -> f32 {
  return 1.0 - exp(-c.distance / globals.fog_distance);
}

// How a chunk shades a colour weighted by its alpha: its grey in the
// chunk's tint, or the colour as it is, fogged toward the horizon.
// It is linear in the colour (shaded), so a weighted sum of colours
// shades as one.
struct Shading {
  // Times the colour's grey, and (a) times the colour;
  grey: vec4f,
  // times its weight.
  weight: vec3f,
}

fn shading_of(c: Chunk) -> Shading {
  let fog = fog_of(c);
  let horizon = fog * globals.horizon.rgb;
  if c.tint == 0u {
    return Shading(vec4f(vec3f(0.0), 1.0 - fog), horizon);
  }
  let tint = (1.0 - fog) * unpack4x8unorm(c.tint).rgb;
  return Shading(vec4f(0.7 * tint, 0.0), 0.3 * tint + horizon);
}

fn shaded(s: Shading, paint: vec4f) -> vec3f {
  return s.grey.a * paint.rgb + s.grey.rgb * dot(paint.rgb, vec3f(0.3, 0.59, 0.11)) + s.weight * paint.a;
}

// The face of a ray, for one face only, so that no pixel is left to none
// or drawn by two (bake's Face::holds): along its largest component if
// that is over (√2 + 1) times the next (TAN), else along the edge of its
// two largest, the smallest left out (seams_fragment); ties to the last.
// Its lattice vector F.
fn face_of(ray: vec3f) -> vec3f {
  let a = abs(ray);
  let least = select(select(2u, 1u, a.y < a.z), 0u, a.x < min(a.y, a.z));
  let most = select(select(2u, 1u, a.y >= a.z), 0u, a.x >= max(a.y, a.z));
  let second = 3u - least - most;
  var f = vec3f(0.0);
  f[most] = sign(ray[most]);
  if a[second] > TAN * a[most] {
    f[second] = sign(ray[second]);
  }
  return f;
}

// The picture, the medium's pictures laid on it (veil_fragment).
@fragment
fn composite_fragment(in: Sky) -> @location(0) vec4f {
  return textureLoad(veil, vec2i(in.position.xy), 0);
}

// |v| for the v in [lo, hi] nearest to 0.
fn closest_to_zero(lo: f32, hi: f32) -> f32 {
  return select(select(0.0, -hi, hi < 0.0), lo, lo > 0.0);
}

// The rays through a box from the eye, `lo` to `hi` across and up, `z`
// along the axis, as a rectangle of face `face`'s tangents, at most its
// corners', near and far. `ahead`: NOWHERE if it is all behind the eye or
// past its face's rays, a hair wider (face_of); BEHIND if it reaches behind
// the eye's plane; else AHEAD.
const NOWHERE = 0u;
const BEHIND = 1u;
const AHEAD = 2u;
struct Rays {
  lo: vec2f,
  hi: vec2f,
  ahead: u32,
}
fn rays_through(face: u32, lo: vec2f, hi: vec2f, z: vec2f) -> Rays {
  var r = Rays(lo, hi, AHEAD);
  if z.y <= 0.0 {
    r.ahead = NOWHERE;
    return r;
  }
  if z.x <= 0.0 {
    r.ahead = BEHIND;
    return r;
  }
  r.lo = min(r.lo / z.x, r.lo / z.y);
  r.hi = max(r.hi / z.x, r.hi / z.y);
  let near = vec2f(closest_to_zero(r.lo.x, r.hi.x), closest_to_zero(r.lo.y, r.hi.y));
  let edge = globals.faces[face].slant.z == 2.0;
  if near.x > TAN * 1.001 || (!edge && near.y > TAN * 1.001) || (edge && near.x + sqrt(2.0) * near.y > 1.001) {
    r.ahead = NOWHERE;
  }
  return r;
}

// A cluster's, from its chunk's place.
fn rays_of(cluster: Cluster) -> Rays {
  let c = chunks[cluster.slot_count >> 8u];
  return rays_through(c.face, c.origin + cluster.min, c.origin + cluster.max, c.distance + vec2f(cluster.near, cluster.far));
}

// A rectangle of face `face`'s tangents on screen, in pixels: its corners
// round it, from (lo, lo), (hi, lo), (hi, hi), (lo, hi); `behind` if one
// is behind the camera, so that it goes off the screen.
struct Quad {
  corners: array<vec2f, 4>,
  behind: bool,
}
fn quad_of(face: u32, lo: vec2f, hi: vec2f) -> Quad {
  let view = transpose(globals.faces[face].view);
  var q: Quad;
  q.behind = false;
  for (var k = 0u; k < 4u; k++) {
    let p = view * vec3f(select(lo.x, hi.x, k == 1u || k == 2u), select(lo.y, hi.y, k >= 2u), 1.0);
    q.behind = q.behind || p.z <= 0.0;
    q.corners[k] = (p.xy * globals.focal / (globals.viewport / 2.0) / p.z * vec2f(0.5, -0.5) + 0.5) * globals.resolution;
  }
  return q;
}

// Where a cluster may draw on screen, in pixels (left, top, right, bottom):
// its rays' quad's box; empty if it draws nowhere, all the screen if it
// reaches behind the eye.
fn screen_box(cluster: Cluster) -> vec4f {
  let r = rays_of(cluster);
  if r.ahead == NOWHERE {
    return vec4f(0.0);
  }
  let q = quad_of(chunks[cluster.slot_count >> 8u].face, r.lo, r.hi);
  if r.ahead == BEHIND || q.behind {
    return vec4f(vec2f(0.0), globals.resolution);
  }
  let c = q.corners;
  let b = vec4f(min(min(c[0], c[1]), min(c[2], c[3])), max(max(c[0], c[1]), max(c[2], c[3])));
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
  let last = level_last(level);
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

// The least depth a cluster's columns may be drawn at, as of
// `level`: of the least w in its box.
fn nearest_depth(cluster: Cluster, level: f32) -> f32 {
  let c = chunks[cluster.slot_count >> 8u];
  let lo = vec3f(c.origin + cluster.min, c.distance + cluster.near);
  return box_depth(c.face, lo, vec3f(c.origin + cluster.max, c.distance + cluster.far), level);
}

// Of a box of face `face` from `lo` to `hi`, relative to the eye.
fn box_depth(face: u32, lo: vec3f, hi: vec3f, level: f32) -> f32 {
  let ahead = globals.faces[face].view[2];
  let w = dot(min(ahead * lo, ahead * hi), vec3f(1.0));
  return select(0.0, flat_depth(level, w), w > 0.0);
}

// The level whose whole voxels a face draws `ahead` along its axis from the
// eye (claims): the finest whose reach holds that layer. A picture of
// medium there takes its depths, as its whole voxels would be: its cells
// are coarser than the whole voxels' drawn there.
fn level_ahead(face: u32, ahead: f32) -> f32 {
  let eye = globals.faces[face].eye.z;
  let spacing = globals.faces[face].axis.w;
  let layer = i32(floor((eye + ahead) / spacing));
  let at = i32(floor(eye / spacing));
  var level = 0;
  while level < globals.lod.z && layer > reach(level, at) {
    level++;
  }
  return f32(level);
}

// Off, every cluster in view is drawn: for tests, as what culling must not change.
override culls = true;

// Whether to draw a cluster in a phase; in the second, whether it shows is
// kept for the next frame.
fn draws_cluster(at: u32, cluster: Cluster, phase: u32) -> bool {
  // None where its chunk's cell is not claimed (place).
  let b = select(vec4f(0.0), screen_box(cluster), chunks[cluster.slot_count >> 8u].plane.w != 0.0);
  if phase == 0u {
    return cluster.visible != 0u && shows(b);
  }
  let visible = shows(b) && !(culls && hidden(b, nearest_depth(cluster, chunks[cluster.slot_count >> 8u].level)));
  clusters[at].visible = u32(visible);
  // The first phase drew those visible before and in view.
  return visible && cluster.visible == 0u;
}

// A thread per cluster of all, of a chunk the frame draws; those left
// where chunks were, which their slot's chunk does not hold, are none.
// Whole voxels' (runs of a picture), its draw: chunks were uploaded near
// to far, at first, so that nearer voxels are mostly drawn first and hide
// what is behind them early. A picture of
// medium's, none, but in the second phase each half counted in its place
// (veil_place), for order_veils to list. None: no vertices.
fn cull(at: u32, phase: u32) -> DrawArgs {
  var draw = DrawArgs(0u, 0u, 0u, 0u);
  if at >= arrayLength(&clusters) {
    return draw;
  }
  let cluster = clusters[at];
  let c = chunks[cluster.slot_count >> 8u];
  if at >= c.whole.y && at < c.veils {
    if phase == 1u {
      for (var half = 0u; half < 2u; half++) {
        let place = veil_place(cluster, c, half);
        if place >= 0 {
          atomicAdd(&veil_places[place], 1u);
        }
      }
    }
  } else if at >= c.whole.x && at < c.whole.y && draws_cluster(at, cluster, phase) {
    draw = DrawArgs(4u, cluster.slot_count & 0xffu, at * 4u, 0u);
  }
  return draw;
}

// A workgroup's draws (cull's, a thread each), those with vertices, in
// its threads' order, listed from its first place in `listed`, and how
// many in `blocks`; scan_draws and pack_draws put them after those of the
// workgroups before.
var<workgroup> kept: array<u32, 64>;
fn list(id: vec3u, n: vec3u, thread: u32, draw: DrawArgs) {
  let group = id.y * n.x + id.x;
  kept[thread] = u32(draw.vertices != 0u);
  workgroupBarrier();
  var rank = 0u;
  for (var k = 0u; k < thread; k++) {
    rank += kept[k];
  }
  if draw.vertices != 0u {
    listed[group * 64u + rank] = draw;
  }
  if thread == 63u {
    blocks[1u + group] = rank + kept[63];
  }
}

// Where each workgroup's draws go (list): after all those of the
// workgroups before, in place of how many it listed; and first, how many
// in all. A thread per 256th of the workgroups, summing its own, then
// the sums before it (Hillis and Steele's scan).
var<workgroup> sums: array<u32, 256>;
@compute @workgroup_size(256)
fn scan_draws(@builtin(local_invocation_index) thread: u32) {
  let groups = arrayLength(&blocks) - 1u;
  let share = (groups + 255u) / 256u;
  let first = min(thread * share, groups);
  let end = min(first + share, groups);
  var own = 0u;
  for (var g = first; g < end; g++) {
    own += blocks[1u + g];
  }
  sums[thread] = own;
  workgroupBarrier();
  for (var d = 1u; d < 256u; d <<= 1u) {
    var before = 0u;
    if thread >= d {
      before = sums[thread - d];
    }
    workgroupBarrier();
    sums[thread] += before;
    workgroupBarrier();
  }
  var at = sums[thread] - own;
  for (var g = first; g < end; g++) {
    let count = blocks[1u + g];
    blocks[1u + g] = at;
    at += count;
  }
  if thread == 255u {
    blocks[0] = sums[255];
  }
}

// A thread per cluster, as cull's: the draws its workgroup listed, where
// scan_draws put them.
@compute @workgroup_size(64)
fn pack_draws(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  let group = id.y * n.x + id.x;
  let groups = arrayLength(&blocks) - 1u;
  if group >= groups {
    return;
  }
  let first = blocks[1u + group];
  let end = select(blocks[0], blocks[2u + group], group + 1u < groups);
  if thread < end - first {
    draws[first + thread] = listed[group * 64u + thread];
  }
}

// Where among the half slabs a ray of its face crosses a picture of
// medium's cluster draws its `half`, numbered from the near plane: the
// finer levels' first, as many as fit between their planes (place), and
// two more, as the finest's nearer one is rounded to its grid; then its
// own level's, from its `clip`, each 2^(L − 1) layers of grid 0 deep (bake's
// slab_layers). -1 if the frame does not draw it: hidden behind what the
// first phase drew, or not its level's.
fn veil_place(cluster: Cluster, c: Chunk, half: u32) -> i32 {
  let first = cluster.layer + f32(half) * cluster.half;
  let b = screen_box(cluster);
  if first <= c.clip || first > c.reach || !shows(b) || (culls && hidden(b, nearest_depth(cluster, level_ahead(c.face, c.distance + cluster.near)))) {
    return -1;
  }
  let eye = i32(floor(globals.faces[c.face].eye.z / globals.faces[c.face].axis.w));
  var place = 0;
  for (var l = globals.medium.w; l < i32(c.level); l++) {
    place += ((reach_of(globals.medium, l, eye) - medium_from(l, eye)) >> u32(l - 1)) + 2;
  }
  return place + i32(ceil((first - c.clip) / cluster.half)) - 1;
}

// Where in veil_draws the pictures at each place go, far ones first, as
// many as cull_rest counted at each; and how many in all.
@compute @workgroup_size(1)
fn scan_veils() {
  let places = arrayLength(&veil_places) / 2u;
  var at = 0u;
  for (var k = places; k > 0u; k--) {
    atomicStore(&veil_places[places + k - 1u], at);
    at += atomicLoad(&veil_places[k - 1u]);
  }
  atomicStore(&veils_drawn, at);
}

// Lists the halves of pictures of medium cull_rest counted where scan_veils says.
@compute @workgroup_size(64)
fn order_veils(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  let at = thread_of(id, n, thread);
  if at >= arrayLength(&clusters) {
    return;
  }
  let cluster = clusters[at];
  let c = chunks[cluster.slot_count >> 8u];
  if at < c.whole.y || at >= c.veils {
    return;
  }
  let places = i32(arrayLength(&veil_places) / 2u);
  for (var half = 0u; half < 2u; half++) {
    let place = veil_place(cluster, c, half);
    if place >= 0 {
      veil_draws[atomicAdd(&veil_places[places + place], 1u)] = DrawArgs(4u, cluster.slot_count & 0xffu, (2u * at + half) * 4u, 0u);
    }
  }
}

// The thread's place among those of its dispatch (compositor.rs's groups).
fn thread_of(id: vec3u, n: vec3u, thread: u32) -> u32 {
  return (id.y * n.x + id.x) * 64u + thread;
}

// A thread per probe: those whose cell shows listed in `found`, once the
// frame's whole voxels are all drawn and the pyramid made of them anew
// (the second phase's lacks what showed only then).
@compute @workgroup_size(64)
fn find_probes(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  let k = thread_of(id, n, thread);
  if k >= atomicLoad(&found[1]) || k >= arrayLength(&probes) {
    return;
  }
  let p = probes[k];
  if probe_shows(p) {
    let at = atomicAdd(&found[0], 1u) + 2u;
    if at < arrayLength(&found) {
      atomicStore(&found[at], p.id);
    }
  }
}

// Whether a hidden cell shows: claimed from where the eye is, and some
// pixel's ray through its box goes past all that is drawn (sees); all
// that may, if its box reaches behind the eye or culling is off.
fn probe_shows(p: Probe) -> bool {
  let eye = globals.faces[p.face].eye.xyz;
  if !claims(p.face, p.level, p.layer, i32(floor(eye.z / globals.faces[p.face].axis.w))) {
    return false;
  }
  let lo = vec3f(p.min - eye.xy, p.depth.x - eye.z);
  let hi = vec3f(p.max - eye.xy, p.depth.y - eye.z);
  let r = rays_through(p.face, lo.xy, hi.xy, vec2f(lo.z, hi.z));
  if r.ahead == NOWHERE {
    return false;
  }
  let q = quad_of(p.face, r.lo, r.hi);
  if r.ahead == BEHIND || q.behind || !culls {
    return true;
  }
  // Rasterising rounds the depth a few ulps either way: ties show.
  return sees(q.corners, box_depth(p.face, lo, hi, f32(p.level)) * (1.0 - exp2(-18.0)));
}

// Whether a pixel whose centre is within `quad` (its corners round it, on
// screen, in pixels) is at least `depth` deep. Down the pyramid from the
// level at which the quad's box spans two texels at most each way, into a
// texel only if its farthest is as far and it meets the quad: one wholly
// within has such a pixel, so it goes down only along the quad's edges.
// Past LOOKS texels it says yes: a probe costs so much at most, and one
// that would cost more is not asked about again.
const LOOKS = 64;
// Texels to look at yet: three more a level at most.
const SEEN = 64;
fn sees(quad: array<vec2f, 4>, depth: f32) -> bool {
  let turn = sign(wedge(quad[1] - quad[0], quad[2] - quad[0]));
  let b = clamp(
    vec4f(min(min(quad[0], quad[1]), min(quad[2], quad[3])), max(max(quad[0], quad[1]), max(quad[2], quad[3]))),
    vec4f(0.0),
    globals.resolution.xyxy,
  );
  if turn == 0.0 || !shows(b) {
    return false;
  }
  let levels = i32(textureNumLevels(farthest));
  let top = clamp(i32(ceil(log2(max(max(b.z - b.x, b.w - b.y), 1.0)))) - 1, 0, levels - 1);
  let texel = f32(2 << u32(top));
  let last = level_last(top);
  let start = min(vec2i(floor(b.xy / texel)), last);
  let end = min(vec2i(ceil(b.zw / texel)) - 1, last);
  var seen: array<vec3i, SEEN>;
  var count = 0;
  for (var y = start.y; y <= end.y; y++) {
    for (var x = start.x; x <= end.x; x++) {
      seen[count] = vec3i(x, y, top);
      count++;
    }
  }
  for (var looks = 0; count > 0; looks++) {
    if looks == LOOKS {
      return true;
    }
    count--;
    let t = seen[count];
    let size = f32(2 << u32(t.z));
    let m = meets(quad, turn, vec2f(t.xy) * size, size);
    if m == OUTSIDE || textureLoad(farthest, t.xy, t.z).r < depth {
      continue;
    }
    if m == WITHIN {
      return true;
    }
    if t.z == 0 {
      for (var k = 0; k < 4; k++) {
        let p = t.xy * 2 + vec2i(k & 1, k >> 1);
        if all(vec2f(p) < globals.resolution) && within(quad, turn, vec2f(p) + 0.5) && textureLoad(depth_in, p, 0) >= depth {
          return true;
        }
      }
      continue;
    }
    let below = level_last(t.z - 1);
    for (var k = 0; k < 4; k++) {
      let c = t.xy * 2 + vec2i(k & 1, k >> 1);
      if all(c <= below) {
        if count == SEEN {
          return true;
        }
        seen[count] = vec3i(c, t.z - 1);
        count++;
      }
    }
  }
  return false;
}

// The last texel of the pyramid's `level`: each level halves the last,
// down to a texel (Targets::new). Asked for one level per thread, Mesa's
// lavapipe gave some threads another level's size.
fn level_last(level: i32) -> vec2i {
  return max(vec2i(textureDimensions(farthest)) >> vec2u(u32(level)), vec2i(1)) - 1;
}

// The z of the cross product of `a` and `b`: its sign, which way `b` turns from `a`.
fn wedge(a: vec2f, b: vec2f) -> f32 {
  return a.x * b.y - a.y * b.x;
}

// Whether point `p` is within `quad`, its corners round it `turn`-wise.
fn within(quad: array<vec2f, 4>, turn: f32, p: vec2f) -> bool {
  for (var e = 0u; e < 4u; e++) {
    if turn * wedge(quad[(e + 1u) % 4u] - quad[e], p - quad[e]) < 0.0 {
      return false;
    }
  }
  return true;
}

// Whether a square, from `corner` `size` each way, is OUTSIDE `quad`
// (all its corners outside one edge), WITHIN it, or else MEETS it, as
// far as that tells.
const OUTSIDE = 0u;
const MEETS = 1u;
const WITHIN = 2u;
fn meets(quad: array<vec2f, 4>, turn: f32, corner: vec2f, size: f32) -> u32 {
  var all_within = true;
  for (var e = 0u; e < 4u; e++) {
    let d = quad[(e + 1u) % 4u] - quad[e];
    var some = false;
    for (var k = 0u; k < 4u; k++) {
      let inside = turn * wedge(d, corner + vec2f(f32(k & 1u), f32(k >> 1u)) * size - quad[e]) >= 0.0;
      some = some || inside;
      all_within = all_within && inside;
    }
    if !some {
      return OUTSIDE;
    }
  }
  return select(MEETS, WITHIN, all_within);
}

@compute @workgroup_size(64)
fn cull_drawn(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  list(id, n, thread, cull(thread_of(id, n, thread), 0u));
}

@compute @workgroup_size(64)
fn cull_rest(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  list(id, n, thread, cull(thread_of(id, n, thread), 1u));
}

// Level 0 draws from NEAR_LAYERS of grid 0 ahead of the eye's (lod.rs's NEAR).
const NEAR_LAYERS = 4;

// The last layer of grid 0 whose cells `level` claims, for an eye in layer
// `eye` (Lod::reach), of `lod`'s Q and coarsest level: 2Q · N layers past
// the last of its own layers of sites not past the eye's; all, for the
// coarsest.
fn reach_of(lod: vec4i, level: i32, eye: i32) -> i32 {
  if level >= lod.z {
    return 0x7fffffff;
  }
  return ((eye >> u32(level)) + 2 * lod.x) << u32(level);
}

// Of whole voxels' cells.
fn reach(level: i32, eye: i32) -> i32 {
  return reach_of(globals.lod, level, eye);
}

// a / b rounded down, b > 0.
fn floor_div(a: i32, b: i32) -> i32 {
  return select(a / b, (a - b + 1) / b, a < 0);
}

// The layer of grid 0 the render distance reaches along face `face`'s
// axis, from an eye in layer `eye`.
fn last_layer(face: u32, eye: i32) -> i32 {
  return eye + i32(ceil(f32(globals.lod.y) / globals.faces[face].axis.w));
}

// Whether a level's cells of layer K along `face` are drawn, its cells N
// = 2^level layers apart, those past `past` and to `to` (Lod::layers): of
// its cells some child, its farthest 2K + max(F, 0) of the finer level,
// past `past`, not past `to` nor the render distance.
fn drawn_layer(face: u32, k: i32, n: i32, past: i32, to: i32, last: i32) -> bool {
  let ahead = i32(globals.faces[face].slant.w);
  let first = floor_div(2 * past - ahead * n, 2 * n) + 1;
  return first * n <= last && first <= k && k * n <= to;
}

// Whether `level` draws its cell of layer K along `face`, for an eye in
// layer `eye` (Lod::layers): claimed, and not all within the finer level's
// claims; not past the render distance.
fn claims(face: u32, level: i32, k: i32, eye: i32) -> bool {
  let n = 1 << u32(level);
  let last = last_layer(face, eye);
  var past = eye + NEAR_LAYERS;
  if level > globals.lod.w {
    past = reach(level - 1, eye);
  }
  return drawn_layer(face, k, n, past, min(reach(level, eye), last + n), last);
}

// The layer of grid 0 past which `level` of the medium's cells draws
// medium (Lod::layers): the finer level's reach, or the near plane, if it
// is the finest that draws it.
fn medium_from(level: i32, eye: i32) -> i32 {
  return select(reach_of(globals.medium, level - 1, eye), eye + NEAR_LAYERS, level <= globals.medium.w);
}

// Whether `level` draws the medium of its cell of layer K along `face`
// (Lod::layers): a layer of cells more each way than it claims, which may
// hold what it draws of a slab of a column (bake's Medium).
fn medium_claims(face: u32, level: i32, k: i32, eye: i32) -> bool {
  let n = 1 << u32(level);
  let last = last_layer(face, eye);
  var to = last + n;
  if level < globals.medium.z {
    to = min(reach_of(globals.medium, level, eye) + n, to);
  }
  return drawn_layer(face, k, n, medium_from(level, eye) - n, to, last);
}

// A thread per slot: its chunk as the frame draws it, from the camera. Its
// plane and origin relative to the eye; its clusters; whether its level
// claims its cell; between which layers past its plane it draws medium,
// none if its level does not pick its cell for it (medium_claims): past
// the finer level's reach (or the near plane), to its own, of the
// medium's cells (veil_fragment); and drawn
// flat, its picture enlarged, each edge out by how far its edge pillars
// reach from its plane times the slant of the rays to it at its nearest, at
// most the face's (compositor.rs's Flat).
@compute @workgroup_size(64)
fn place(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  let slot = thread_of(id, n, thread);
  if slot >= arrayLength(&records) {
    return;
  }
  let r = records[slot];
  let f = globals.faces[r.face];
  let eye = f.eye.xyz;
  let layer = i32(floor(eye.z / f.axis.w));
  // Its layer of cells, K, from its middle (bake's CellChunk::centre).
  let m = 1 << u32(r.level - r.atoms);
  let k = (r.centre - (m - 1) * i32(f.eye.w)) / (2 * m);
  var c: Chunk;
  c.origin = r.origin - eye.xy;
  c.distance = r.depth - eye.z;
  c.face = r.face;
  c.level = f32(r.level);
  c.tint = r.tint;
  c.half_layer = r.half_layer;
  c.clip = f32(2 * (medium_from(r.level, layer) >> u32(r.atoms)) - r.centre);
  let reach_out = r.plane.w;
  let nearest = r.plane.x + min(r.plane.y * r.min.x, r.plane.y * r.max.x) + min(r.plane.z * r.min.y, r.plane.z * r.max.y);
  let z = c.distance + nearest - reach_out;
  let far = max(abs(c.origin + r.min), abs(c.origin + r.max));
  let slant = select(f.slant.xy, min(far / z, f.slant.xy), z > 0.0);
  let half = (r.max - r.min) / 2.0;
  c.scale = select(vec2f(1.0), 1.0 + slant * reach_out / half, half > vec2f(0.0));
  c.offset = (r.min + r.max) / 2.0 * (1.0 - c.scale);
  c.plane = vec4f(r.plane.xyz, select(0.0, 1.0, claims(r.face, r.level, k, layer)));
  c.whole = r.whole;
  c.veils = r.veils;
  c.reach = 3.0e38;
  if r.level < globals.medium.z {
    c.reach = f32(2 * (reach_of(globals.medium, r.level, layer) >> u32(r.atoms)) - r.centre);
  }
  if !medium_claims(r.face, r.level, k, layer) {
    c.clip = 3.0e38;
  }
  made[slot] = c;
}

// A thread per word written: where its run puts it (the last run whose
// words come no later).
@compute @workgroup_size(64)
fn scatter(@builtin(workgroup_id) id: vec3u, @builtin(num_workgroups) n: vec3u, @builtin(local_invocation_index) thread: u32) {
  let k = thread_of(id, n, thread);
  if k >= runs[0].y {
    return;
  }
  var lo = 1u;
  var hi = runs[0].x + 1u;
  while hi - lo > 1u {
    let mid = (lo + hi) / 2u;
    if runs[mid].y <= k {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  into[runs[lo].x + k - runs[lo].y] = written[k];
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

// The seams between faces, over the picture: where a ray is as near the
// axis of its face (face_of) as of the next: its largest component m1,
// the next m2 and the least m3, the axis's m1 against the edge's (m1 +
// m2) / √2, and the edge's against the other edge's, (m1 + m3) / √2.
@fragment
fn seams_fragment(in: Sky) -> @location(0) vec4f {
  let a = abs(ray_of(globals.sky_view, in.position));
  let m1 = max(max(a.x, a.y), a.z);
  let m3 = min(min(a.x, a.y), a.z);
  let m2 = a.x + a.y + a.z - m1 - m3;
  let s = sqrt(0.5);
  let near = vec3f(m1, (m1 + m2) * s, (m1 + m3) * s);
  let best = max(near.x, near.y);
  let gap = best - select(near.y, max(near.x, near.z), near.y >= near.x);
  // Pixels to the seam, near it; the seam 1.5 points wide.
  let pixels = gap / max(fwidth(gap), 1e-9);
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

