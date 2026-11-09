// compositor.ts in WGSL, drawing only the clusters of splats that may show:
// each splat names its chunk's slot in `chunks`, which gives its face and
// plane relative to the camera. Per frame, cull_drawn lists draws
// of the clusters that showed last frame, and splat_vertex draws them; their depth
// is reduced to a pyramid of farthest depths (reduce_depth, reduce), and
// cull_rest tests every cluster against it: those that showed are kept for
// the next frame, and those newly showing drawn now.

struct FaceView {
  // Columns: the camera's right, up and forward axes, in the face's coordinates.
  view: mat3x3f,
  _pad: vec4f,
}

struct Globals {
  viewport: vec2f, // CSS pixels (points)
  resolution: vec2f, // device pixels
  focal: f32,
  fog_distance: f32,
  _pad: f32,
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
  tint: u32, // RGBA to paint its splats in, shaded as their voxels are; 0 for their own
  half_layer: f32, // a half layer of its grid: the unit of its splats' depths
  _pad: f32,
}

// Splats of one chunk near each other on its plane: `first` and the next
// `count` (low byte of `slot_count`, its chunk's slot above), the box of
// their outlines along the axis relative to the chunk's origin, whether
// any showed in the last frame, how far past the plane they are, nearest
// and farthest, and the widest's half width.
struct Cluster {
  min: vec2f,
  max: vec2f,
  first: u32,
  slot_count: u32,
  visible: u32,
  near: f32,
  far: f32,
  wide: f32,
}

// A cluster's draw: its splats, instances of splat_vertex.
struct DrawArgs {
  vertices: u32,
  instances: u32,
  first_vertex: u32,
  first_instance: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> chunks: array<Chunk>;
// Culling: the frame's chunks' clusters, [first, first + count) each, and
// a draw for each to draw.
@group(0) @binding(4) var<storage, read_write> clusters: array<Cluster>;
@group(0) @binding(5) var<storage, read> candidates: array<vec2u>;
@group(0) @binding(6) var<storage, read_write> draws: array<DrawArgs>;
@group(0) @binding(7) var<storage, read_write> drawn: atomic<u32>;
@group(0) @binding(8) var farthest: texture_2d<f32>;
// The pyramid: level 0 from the depth buffer, each next from the last.
@group(0) @binding(10) var depth_in: texture_depth_2d;
@group(0) @binding(11) var level_in: texture_2d<f32>;
@group(0) @binding(12) var level_out: texture_storage_2d<r32float, write>;
// Showing the frame.
@group(0) @binding(13) var picture: texture_2d<f32>;
// Blended: the see-through splats' colours, weighted, and how much of what
// is behind them they let through (see_through_fragment).
@group(0) @binding(14) var blended: texture_2d<f32>;
@group(0) @binding(15) var through: texture_2d<f32>;

// The depth of a splat of a chunk of `level`, its site `distance` from the
// eye: finer levels in front (lod.rs), and within a level the nearer site,
// which, voxels being the lattice's Voronoi cells, is the one a ray meets
// first. Level L's depths are [2^(L - 64), 2^(L - 63)), HALFWAY halfway:
// any distance in the view's scale keeps f32's steps far below a voxel.
// Where faces overlap by a hair (splat_fragment), the first is in front.
const HALFWAY = 1000.0;
fn depth_of(level: f32, distance: f32, face: u32) -> f32 {
  return exp2(level - 64.0) * (1.0 + distance / (distance + HALFWAY)) * (1.0 + f32(face) * exp2(-21.0));
}

struct Splat {
  @builtin(position) position: vec4f,
  @location(0) corner: vec2f,
  // Where on its chunk's plane, over the plane's distance: the ray's tangents in face coordinates.
  @location(1) ray: vec2f,
  @location(2) @interpolate(flat) slant: f32,
  // Its alpha its voxel's: of the rays through its site, how many it stops.
  @location(3) @interpolate(flat) colour: vec4f,
  @location(4) @interpolate(flat) splat: u32,
  @location(5) @interpolate(flat) distance: f32,
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
fn splat_vertex(
  @builtin(vertex_index) vertex: u32,
  @builtin(instance_index) instance: u32,
  @location(0) splat: vec4f,
  @location(1) paint: vec4f,
) -> Splat {
  let c = chunks[vertex >> 2u];
  let face = globals.faces[c.face];
  var out: Splat;
  out.corner = vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u)) * 2.0 - 1.0;
  let site = vec3f(c.origin + splat.xy, c.distance + splat.w * c.half_layer);
  let size = vec2f(splat.z * (1.0 + abs(site.x / site.z)), splat.z * sqrt(2.0));
  out.slant = size.x / (sqrt(2.0) * size.y);
  out.splat = instance;
  out.distance = length(site);
  let fog = 1.0 - exp(-c.distance / globals.fog_distance);
  let tinted = unpack4x8unorm(c.tint).rgb * (0.3 + 0.7 * dot(paint.rgb, vec3f(0.3, 0.59, 0.11)));
  out.colour = vec4f(mix(select(paint.rgb, tinted, c.tint != 0u), globals.horizon.rgb, fog), paint.a);
  let at = site + vec3f(out.corner * size, 0.0);
  out.ray = at.xy / at.z;
  let p = transpose(face.view) * at;
  out.position = vec4f(p.xy * globals.focal / (globals.viewport / 2.0), depth_of(c.level, length(site), c.face) * p.z, p.z);
  return out;
}

// A number in [0, 1) as if at random, for pixel `p` and splat `s`.
fn chance(p: vec2u, s: u32) -> f32 {
  var h = p.x * 0x8da6b343u ^ p.y * 0xd8163841u ^ s * 0xcb1ab31fu;
  h = (h ^ (h >> 16u)) * 0x7feb352du;
  h = (h ^ (h >> 15u)) * 0x846ca68bu;
  return f32((h ^ (h >> 16u)) >> 8u) / 16777216.0;
}

// Whether the pixel is in the splat's hexagon, and its face draws its ray,
// by the ray each splat interpolates (working it out from the pixel costs
// more). Rounding differs between splats, so faces overlap by a hair: no
// pixel is left to neither.
fn covers(in: Splat) -> bool {
  let t = in.ray;
  return abs(in.corner.x) * in.slant + abs(in.corner.y) < 1.0 && abs(t.x) + sqrt(2.0) * abs(t.y) <= 1.001;
}

// By chance: a pixel's ray stops at a splat as often as its voxel stops
// rays, by a chance of the pixel and splat. The depth test leaves the
// nearest it stops at, so it sees through each see-through voxel, by a
// chance of its own, in whatever order they are drawn. A whole voxel's
// alpha is 1: it always stops.
@fragment
fn splat_fragment(in: Splat) -> @location(0) vec4f {
  if !covers(in) || chance(vec2u(in.position.xy), in.splat) >= in.colour.a {
    discard;
  }
  return vec4f(in.colour.rgb, 1.0);
}

// Blended, first the whole voxels, with depth, as by chance.
@fragment
fn whole_fragment(in: Splat) -> @location(0) vec4f {
  if !covers(in) || in.colour.a < 1.0 {
    discard;
  }
  return vec4f(in.colour.rgb, 1.0);
}

struct Blended {
  @location(0) colour: vec4f,
  @location(1) through: vec4f,
}

// Then the see-through ones in front of them, in any order (weighted
// blended order-independent transparency, McGuire and Bavoil 2013): what
// they let through is exact, the product of 1 − alpha; the colour, each
// one's weighted by its alpha and by how near it is, so that the nearer
// show over the farther. The weight keeps the sum within a half float.
@fragment
fn see_through_fragment(in: Splat) -> Blended {
  if !covers(in) || in.colour.a >= 1.0 {
    discard;
  }
  let a = in.colour.a;
  let weight = a * clamp(1e3 * pow(in.distance, -1.5), 1e-3, 1e3);
  return Blended(vec4f(in.colour.rgb, 1.0) * weight, vec4f(a));
}

// Their colour over the picture, as much as they stop.
@fragment
fn composite_fragment(in: Sky) -> @location(0) vec4f {
  let p = vec2i(in.position.xy);
  let through = textureLoad(through, p, 0).r;
  if through >= 1.0 {
    discard;
  }
  let sum = textureLoad(blended, p, 0);
  return vec4f(sum.rgb / max(sum.a, 1e-6), 1.0 - through);
}

// |v| for the v in [lo, hi] nearest to 0.
fn closest_to_zero(lo: f32, hi: f32) -> f32 {
  return select(select(0.0, -hi, hi < 0.0), lo, lo > 0.0);
}

// Where a cluster may draw on screen, in pixels (left, top, right, bottom),
// within its face's rays, a hair wider (splat_fragment); empty if it draws
// nowhere.
fn screen_box(cluster: Cluster) -> vec4f {
  let c = chunks[cluster.slot_count >> 8u];
  // The rays through its splats, as tangents: each is drawn at its depth,
  // wider by its slant across (splat_vertex), at most the box's corners'.
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
  // Each level halves the last (Targets::new). Asked for one level per
  // thread, Mesa's lavapipe gave some threads another level's size.
  let last = (vec2i(textureDimensions(farthest)) >> vec2u(u32(level))) - 1;
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

// Off, every cluster in view is drawn: for tests, as what culling must not change.
override culls = true;

// Whether to draw a cluster in a phase; in the second, whether it shows is
// kept for the next frame.
fn draws_cluster(at: u32, cluster: Cluster, phase: u32) -> bool {
  let b = screen_box(cluster);
  if phase == 0u {
    return cluster.visible != 0u && shows(b);
  }
  let c = chunks[cluster.slot_count >> 8u];
  // Its nearest splat's distance is no less than how far ahead it is.
  let depth = depth_of(c.level, max(c.distance + cluster.near, 0.0), c.face);
  let visible = shows(b) && !(culls && hidden(b, depth));
  clusters[at].visible = u32(visible);
  // The first phase drew those visible before and in view.
  return visible && cluster.visible == 0u;
}

// A workgroup per chunk in the frame, a thread per cluster. Chunks come
// near to far and their clusters nearest first (see Compositor::upload),
// and draws are listed about in that order, so that nearer splats are
// mostly drawn first and hide what is behind them early.
fn cull(chunk: u32, thread: u32, phase: u32) {
  let range = candidates[chunk];
  for (var k = thread; k < range.y; k += 64u) {
    let cluster = clusters[range.x + k];
    if draws_cluster(range.x + k, cluster, phase) {
      draws[atomicAdd(&drawn, 1u)] = DrawArgs(4u, cluster.slot_count & 0xffu, (cluster.slot_count >> 8u) * 4u, cluster.first);
    }
  }
}

@compute @workgroup_size(64)
fn cull_drawn(@builtin(workgroup_id) chunk: vec3u, @builtin(local_invocation_index) thread: u32) {
  cull(chunk.x, thread, 0u);
}

@compute @workgroup_size(64)
fn cull_rest(@builtin(workgroup_id) chunk: vec3u, @builtin(local_invocation_index) thread: u32) {
  cull(chunk.x, thread, 1u);
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
