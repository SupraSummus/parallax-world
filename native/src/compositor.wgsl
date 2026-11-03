// compositor.ts in WGSL: each splat instance names its chunk's slot in
// `chunks`, which gives its face, plane and clip relative to the camera.

struct FaceView {
  // Columns: the camera's right, up and forward axes, in the face's coordinates.
  view: mat3x3f,
  _pad: vec4f,
}

struct Globals {
  viewport: vec2f, // CSS pixels (points)
  resolution: vec2f, // device pixels
  offset: vec2f, // the view's top-left corner in the framebuffer, device pixels
  focal: f32,
  fog_distance: f32,
  sky_top: vec4f,
  horizon: vec4f,
  sky_view: mat3x3f, // the camera's axes in world coordinates
  depth_scale: f32, // see depth_of
  _pad1: f32,
  sky_fog: f32,
  _pad: f32,
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

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> chunks: array<Chunk>;

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
  let s = (position.xy - globals.offset) / globals.resolution - 0.5;
  return view * vec3f(vec2f(s.x, -s.y) * globals.viewport / globals.focal, 1.0);
}

// Painting order is depth: layer k of L at 2^((k - L) · depth_scale),
// near to far, and within a chunk by `order`, in [0, 1), nearer the later
// a splat is painted (see Compositor::prepare), up to the next layer's.
// Exponential, so each layer has about as many depths as a whole [½, 1)
// would linearly, enough for every splat of the largest chunks.
fn depth_of(layer: f32, order: f32) -> f32 {
  return layer * exp2(order * globals.depth_scale);
}

@vertex
fn splat_vertex(
  @builtin(vertex_index) vertex: u32,
  @location(0) splat: vec4f,
  @location(1) paint: vec4f,
  @location(2) chunk: u32,
  @location(3) order: f32,
) -> Splat {
  let c = chunks[chunk];
  let face = globals.faces[c.face];
  var out: Splat;
  out.corner = vec2f(f32(vertex & 1u), f32(vertex >> 1u)) * 2.0 - 1.0;
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
