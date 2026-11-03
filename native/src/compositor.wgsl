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
  sky_depth: f32,
  half_layer: f32, // depth from one layer to its grown splats
  sky_fog: f32,
  _pad: f32,
  faces: array<FaceView, 12>,
}

struct Chunk {
  origin: vec2f, // top-left corner of what is drawn, relative to the camera, face coordinates
  extent: vec2f,
  distance: f32, // of the centre plane, along the face's axis
  face: u32,
  depth: f32, // the layer's depth in (0, 1)
  _pad: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> chunks: array<Chunk>;

struct Splat {
  @builtin(position) position: vec4f,
  @location(0) corner: vec2f,
  @location(1) @interpolate(flat) slant: f32,
  @location(2) @interpolate(flat) colour: vec3f,
  @location(3) @interpolate(flat) chunk: u32,
}

fn ray_of(view: mat3x3f, position: vec4f) -> vec3f {
  let s = (position.xy - globals.offset) / globals.resolution - 0.5;
  return view * vec3f(vec2f(s.x, -s.y) * globals.viewport / globals.focal, 1.0);
}

// `chunk`'s top bit marks grown splats, drawn half a layer behind the painted.
@vertex
fn splat_vertex(@builtin(vertex_index) vertex: u32, @location(0) splat: vec4f, @location(1) paint: vec4f, @location(2) chunk: u32) -> Splat {
  let c = chunks[chunk & 0x7fffffffu];
  let face = globals.faces[c.face];
  var out: Splat;
  out.corner = vec2f(f32(vertex & 1u), f32(vertex >> 1u)) * 2.0 - 1.0;
  out.slant = splat.z / (sqrt(2.0) * splat.w);
  out.colour = paint.rgb;
  out.chunk = chunk & 0x7fffffffu;
  let p = transpose(face.view) * vec3f(c.origin + splat.xy + out.corner * splat.zw, c.distance);
  let depth = c.depth + select(0.0, globals.half_layer, chunk >= 0x80000000u);
  out.position = vec4f(p.xy * globals.focal / (globals.viewport / 2.0), depth * p.z, p.z);
  return out;
}

@fragment
fn splat_fragment(in: Splat) -> @location(0) vec4f {
  if abs(in.corner.x) * in.slant + abs(in.corner.y) >= 1.0 {
    discard;
  }
  let c = chunks[in.chunk];
  let face = globals.faces[c.face];
  let ray = ray_of(face.view, in.position);
  if abs(ray.x) + sqrt(2.0) * abs(ray.y) > ray.z * 1.001 {
    discard;
  }
  let p = ray.xy / ray.z * c.distance;
  if p.x < c.origin.x || p.x >= c.origin.x + c.extent.x || p.y > c.origin.y || p.y <= c.origin.y - c.extent.y {
    discard;
  }
  let fog = 1.0 - exp(-c.distance / globals.fog_distance);
  return vec4f(mix(in.colour, globals.horizon.rgb, fog), 1.0);
}

struct Sky {
  @builtin(position) position: vec4f,
}

@vertex
fn sky_vertex(@builtin(vertex_index) vertex: u32) -> Sky {
  let corner = vec2f(f32(vertex & 1u), f32(vertex >> 1u)) * 2.0 - 1.0;
  return Sky(vec4f(corner, globals.sky_depth, 1.0));
}

@fragment
fn sky_fragment(in: Sky) -> @location(0) vec4f {
  let ray = ray_of(globals.sky_view, in.position);
  let elevation = ray.y / length(ray.xz);
  let rgb = mix(globals.horizon.rgb, globals.sky_top.rgb, clamp(elevation / 0.625, 0.0, 1.0));
  return vec4f(mix(rgb, globals.horizon.rgb, globals.sky_fog), 1.0);
}
