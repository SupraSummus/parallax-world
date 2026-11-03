// compositor.ts in WGSL, drawing only the clusters of splats that may show:
// each splat names its chunk's slot in `chunks`, which gives its face,
// plane and clip relative to the camera. Per frame, cull_drawn lists draws
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
  depth_scale: f32, // see depth_of
  sky_fog: f32,
  sky_top: vec4f,
  horizon: vec4f,
  sky_view: mat3x3f, // the camera's axes in world coordinates
  faces: array<FaceView, 12>,
}

struct Chunk {
  origin: vec2f, // top-left corner of what is drawn, relative to the camera, face coordinates
  extent: vec2f,
  distance: f32, // of the centre plane, along the face's axis
  face: u32,
  depth: f32, // its layer's: see depth_of
  _pad: f32,
}

// Splats of one chunk near each other on its plane: `first` and the next
// `count` (low byte of `slot_count`, its chunk's slot above), their box
// relative to the chunk's origin, the least order among them, and whether
// any showed in the last frame.
struct Cluster {
  min: vec2f,
  max: vec2f,
  first: u32,
  slot_count: u32,
  order: f32,
  visible: u32,
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

// Painting order is depth: layer k of L at 2^((k - L) · depth_scale),
// near to far, and within a chunk by `order`, in [0, 1), nearer the later
// a splat is painted (see Compositor::prepare), up to the next layer's.
// Exponential, so each layer has about as many depths as a whole [½, 1)
// would linearly, enough for every splat of the largest chunks.
fn depth_of(layer: f32, order: f32) -> f32 {
  return layer * exp2(order * globals.depth_scale);
}

struct Splat {
  @builtin(position) position: vec4f,
  @location(0) corner: vec2f,
  // Where on its chunk's plane, over the plane's distance: the ray's tangents in face coordinates.
  @location(1) ray: vec2f,
  @location(2) @interpolate(flat) slant: f32,
  @location(3) @interpolate(flat) colour: vec3f,
  // The chunk's left, top, right and bottom, as tangents too, a hair wider.
  @location(4) @interpolate(flat) clip: vec4f,
}

fn ray_of(view: mat3x3f, position: vec4f) -> vec3f {
  let s = position.xy / globals.resolution - 0.5;
  return view * vec3f(vec2f(s.x, -s.y) * globals.viewport / globals.focal, 1.0);
}

// A splat instance (GpuSplat): its hexagon's centre relative to its chunk's
// origin and half its width and height, its colour and its order in the
// chunk. Its chunk's slot comes as the first vertex's index over 4.
@vertex
fn splat_vertex(
  @builtin(vertex_index) vertex: u32,
  @location(0) splat: vec4f,
  @location(1) paint: vec4f,
  @location(2) order: f32,
) -> Splat {
  let c = chunks[vertex >> 2u];
  let face = globals.faces[c.face];
  var out: Splat;
  out.corner = vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u)) * 2.0 - 1.0;
  out.slant = splat.z / (sqrt(2.0) * splat.w);
  let fog = 1.0 - exp(-c.distance / globals.fog_distance);
  out.colour = mix(paint.rgb, globals.horizon.rgb, fog);
  let at = c.origin + splat.xy + out.corner * splat.zw;
  out.ray = at / c.distance;
  out.clip = vec4f(c.origin.x, c.origin.y, c.origin.x + c.extent.x, c.origin.y - c.extent.y) / c.distance + vec4f(-1.0, 1.0, 1.0, -1.0) * 1e-4;
  let p = transpose(face.view) * vec3f(at, c.distance);
  out.position = vec4f(p.xy * globals.focal / (globals.viewport / 2.0), depth_of(c.depth, order) * p.z, p.z);
  return out;
}

// The face draws its own rays, a chunk its part of the plane, by the ray
// each splat interpolates (working it out from the pixel costs more).
// Rounding differs between splats, so faces and chunks overlap by a hair:
// no pixel is left to neither.
@fragment
fn splat_fragment(in: Splat) -> @location(0) vec4f {
  if abs(in.corner.x) * in.slant + abs(in.corner.y) >= 1.0 {
    discard;
  }
  let t = in.ray;
  if abs(t.x) + sqrt(2.0) * abs(t.y) > 1.001 {
    discard;
  }
  if t.x < in.clip.x || t.x >= in.clip.z || t.y > in.clip.y || t.y <= in.clip.w {
    discard;
  }
  return vec4f(in.colour, 1.0);
}

// |v| for the v in [lo, hi] nearest to 0.
fn closest_to_zero(lo: f32, hi: f32) -> f32 {
  return select(select(0.0, -hi, hi < 0.0), lo, lo > 0.0);
}

// Where a cluster may draw on screen, in pixels (left, top, right, bottom),
// as its chunk is clipped to its cell and its face to the face's rays, both
// a hair wider (splat_fragment); empty if it draws nowhere.
fn screen_box(cluster: Cluster) -> vec4f {
  let c = chunks[cluster.slot_count >> 8u];
  let hair = 1e-4 * c.distance;
  let lo = max(c.origin + cluster.min, vec2f(c.origin.x, c.origin.y - c.extent.y) - hair);
  let hi = min(c.origin + cluster.max, vec2f(c.origin.x + c.extent.x, c.origin.y) + hair);
  if any(lo >= hi) {
    return vec4f(0.0);
  }
  if closest_to_zero(lo.x, hi.x) + sqrt(2.0) * closest_to_zero(lo.y, hi.y) > 1.001 * c.distance {
    return vec4f(0.0);
  }
  let view = transpose(globals.faces[c.face].view);
  var b = vec4f(globals.resolution, vec2f(0.0));
  for (var k = 0u; k < 4u; k++) {
    let p = view * vec3f(select(lo.x, hi.x, (k & 1u) != 0u), select(lo.y, hi.y, k >= 2u), c.distance);
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
  let last = vec2i(textureDimensions(farthest, level)) - 1;
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

// Whether to draw a cluster in a phase; in the second, whether it shows is
// kept for the next frame.
fn draws_cluster(at: u32, cluster: Cluster, phase: u32) -> bool {
  let b = screen_box(cluster);
  if phase == 0u {
    return cluster.visible != 0u && shows(b);
  }
  let depth = depth_of(chunks[cluster.slot_count >> 8u].depth, cluster.order);
  let visible = shows(b) && !hidden(b, depth);
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
