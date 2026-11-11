// slabs.rs: a ray walks its face's slabs, near to far, and stops at the
// first column whose run it meets: one for every factor × factor pixels,
// each pixel then testing its own ray against what they met (resolve_fragment).

struct FaceView {
  view: mat3x3f,
  _pad: vec4f,
}

// compositor.wgsl's.
struct Globals {
  viewport: vec2f,
  resolution: vec2f,
  focal: f32,
  fog_distance: f32,
  _pad: f32,
  sky_fog: f32,
  sky_top: vec4f,
  horizon: vec4f,
  sky_view: mat3x3f,
  faces: array<FaceView, 12>,
}

// A band of a face's slabs, all of one grid: the eye in its lattice
// coordinates (a, b, c) relative to its first column and slab, and its
// grid's spacing; the layers it holds, relative likewise; how thick its
// slabs are; its columns across and rows, and where its texels begin.
struct Band {
  eye: vec4f,
  start: f32,
  end: f32,
  thickness: f32,
  blocks: u32,
  columns: u32,
  rows: u32,
  slabs: u32,
  offset: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> bands: array<Band>;
@group(0) @binding(2) var<storage, read> texels: array<vec2u>;
// By face: its first band and how many.
@group(0) @binding(3) var<storage, read> face_bands: array<vec2u, 12>;
// Per band, whether each block of BLOCK × BLOCK columns (and a column
// around) holds a voxel: in any of its slabs, then in each.
@group(0) @binding(4) var<storage, read> blocks: array<u32>;

const BLOCK = 8.0;

const LAYER = 0.70710678;
const EMPTY = 0xffffffffu;

// `at` is where in the target, (0, 0) its top left, (1, 1) its bottom right.
struct Screen {
  @builtin(position) position: vec4f,
  @location(0) at: vec2f,
}

@vertex
fn screen_vertex(@builtin(vertex_index) vertex: u32) -> Screen {
  let corner = vec2f(f32(vertex & 1u), f32(vertex >> 1u));
  return Screen(vec4f(corner * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), corner);
}

fn sky(ray: vec3f) -> vec3f {
  let elevation = ray.y / length(ray.xz);
  let rgb = mix(globals.horizon.rgb, globals.sky_top.rgb, clamp(elevation / 0.625, 0.0, 1.0));
  return mix(rgb, globals.horizon.rgb, globals.sky_fog);
}

// The column (a, b), a and b of one parity, nearest (a, b) on the plane:
// x = a · LAYER, y = b.
fn column(p: vec2f) -> vec2f {
  let even = 2.0 * round(p / 2.0);
  let odd = 2.0 * floor(p / 2.0) + 1.0;
  let de = (p - even) * vec2f(LAYER, 1.0);
  let dodd = (p - odd) * vec2f(LAYER, 1.0);
  return select(odd, even, dot(de, de) <= dot(dodd, dodd));
}

// A number in [0, 1) as if at random, for pixel `p` and texel `t` (compositor.wgsl's chance).
fn chance(p: vec2u, t: u32) -> f32 {
  var h = p.x * 0x8da6b343u ^ p.y * 0xd8163841u ^ t * 0xcb1ab31fu;
  h = (h ^ (h >> 16u)) * 0x7feb352du;
  h = (h ^ (h >> 15u)) * 0x846ca68bu;
  return f32((h ^ (h >> 16u)) >> 8u) / 16777216.0;
}

// A pixel's ray: its face, its direction in the camera's axes, and per
// layer along the face's axis how far it goes in a and in b.
struct Ray {
  face: u32,
  camera: vec3f,
  slope: vec2f,
  pixel: vec2u,
}

fn ray_through(pixel: vec2f) -> Ray {
  let s = pixel / globals.resolution - 0.5;
  let camera = vec3f(vec2f(s.x, -s.y) * globals.viewport / globals.focal, 1.0);
  // The face whose axis is nearest the ray: the neighbour direction
  // (±1, ±1, 0) along its two largest components (bake's lattices: faces
  // 4m to 4m + 3 leave out component [1, 0, 2][m], and their signs s, t
  // of the next two are (+, +), (−, +), (+, −), (−, −)).
  let world = globals.sky_view * camera;
  let size = abs(world);
  let least = select(select(2u, 0u, size.x <= size.z), select(2u, 1u, size.y <= size.z), size.y <= size.x);
  let group = select(select(2u, 1u, least == 0u), 0u, least == 1u);
  let next = vec2f(world[(least + 1u) % 3u], world[(least + 2u) % 3u]);
  let face = group * 4u + select(0u, 1u, next.x < 0.0) + select(0u, 2u, next.y < 0.0);
  let d = globals.faces[face].view * camera;
  return Ray(face, camera, vec2f(d.x / d.z, d.y / d.z * LAYER), vec2u(pixel));
}

// What a ray meets first: its band, the texel of its column and slab
// (an index into texels), and the layer where it meets it, in the band's
// coordinates; band NONE if nothing.
struct Hit {
  band: u32,
  texel: u32,
  at: f32,
}

const NONE = 0xffffffffu;

// Whether a voxel stops a pixel's ray: as often as it stops rays, by chance.
fn stops(ray: Ray, texel: u32, paint: u32) -> bool {
  return chance(ray.pixel, texel) < unpack4x8unorm(paint).a;
}

fn march(ray: Ray) -> Hit {
  let slope = ray.slope;
  // Steps of at most a column's half width either way.
  let most = 1.0 / max(max(abs(slope.x), abs(slope.y)), 1e-3);
  let range = face_bands[ray.face];
  for (var k = range.x; k < range.x + range.y; k++) {
    let band = bands[k];
    let eye = band.eye.xyz;
    let rows = i32(band.rows);
    let columns = i32(band.columns);
    var c = max(band.start, eye.z);
    let across = (band.columns + 7u) / 8u;
    let up = (band.rows + 7u) / 8u;
    // A block is 2 · BLOCK wide in a, BLOCK high in b.
    let size = vec2f(2.0 * BLOCK, BLOCK);
    for (var steps = 0; c < band.end && steps < 4096; steps++) {
      let slab = floor(c / band.thickness);
      let slab_end = min((slab + 1.0) * band.thickness, band.end);
      // The block the ray is in, and the layer where it leaves it.
      let p = eye.xy + slope * (c - eye.z);
      let block = floor(p / size);
      let exits = (select(block, block + 1.0, slope > vec2f(0.0)) * size - p) / slope;
      let leaves = c + max(min(select(exits.x, 1e9, slope.x == 0.0), select(exits.y, 1e9, slope.y == 0.0)), 1e-3);
      if any(block < vec2f(0.0)) || block.x >= f32(across) || block.y >= f32(up) {
        c = min(leaves, band.end);
        continue;
      }
      let at = u32(block.y) * across + u32(block.x);
      if blocks[band.blocks + at] == 0u {
        c = min(leaves, band.end);
        continue;
      }
      if blocks[band.blocks + (u32(slab) + 1u) * across * up + at] == 0u {
        c = min(leaves, slab_end);
        continue;
      }
      let next = min(c + most, slab_end);
      let m = (c + next) * 0.5;
      let col = column(eye.xy + slope * (m - eye.z));
      let j = i32(col.y);
      let i = (i32(col.x) - (j & 1)) / 2;
      if col.x >= 0.0 && col.y >= 0.0 && i < columns && j < rows {
        let texel = band.offset + (u32(slab) * band.rows + u32(j)) * band.columns + u32(i);
        let t = texels[texel];
        if t.y != EMPTY {
          // A voxel reaches a layer before and after its site.
          let first = slab * band.thickness + f32(t.y & 0xffffu) - 1.0;
          let last = slab * band.thickness + f32(min(t.y >> 16u, 0xfffeu)) + 1.0;
          if first < next && last > c && stops(ray, texel, t.x) {
            return Hit(k, texel, max(c, first));
          }
        }
      }
      c = next;
    }
  }
  return Hit(NONE, 0u, 0.0);
}

// Traced at 1 / factor of the resolution each way: what the ray through
// the middle of each factor × factor pixels meets (sample_fragment); then
// each pixel takes the nearest of the runs its four nearest samples met
// that its own ray meets too (resolve_fragment). A pixel none of them
// covers takes its nearest sample's, unless one of them met nothing.
override factor = 4u;
// What each sample met: where its column's middle is from the eye, in
// (a, b), and its run's first and last layers from the eye
// (sample_fragment), all in its band's units; and its texel, its band,
// and its band's spacing (as f32 bits).
@group(0) @binding(5) var sample_runs: texture_2d<f32>;
@group(0) @binding(6) var sample_hits: texture_2d<u32>;

struct Sample {
  @location(0) run: vec4f,
  @location(1) hit: vec4u,
}

@fragment
fn sample_fragment(in: Screen) -> Sample {
  let hit = march(ray_through((floor(in.position.xy) + 0.5) * f32(factor)));
  if hit.band == NONE {
    return Sample(vec4f(0.0), vec4u(NONE, NONE, 0u, 0u));
  }
  let band = bands[hit.band];
  let n = hit.texel - band.offset;
  let per = band.rows * band.columns;
  let slab = f32(n / per);
  let j = (n % per) / band.columns;
  let i = (n % per) % band.columns;
  let middle = vec2f(f32(2u * i + (j & 1u)), f32(j));
  let t = texels[hit.texel];
  let eye = band.eye.xyz;
  let first = max(max(slab * band.thickness + f32(t.y & 0xffffu) - 1.0, band.start), eye.z);
  let last = min(slab * band.thickness + f32(min(t.y >> 16u, 0xfffeu)) + 1.0, band.end);
  return Sample(vec4f(middle - eye.xy, first - eye.z, last - eye.z), vec4u(hit.texel, hit.band, bitcast<u32>(band.eye.w), 0u));
}

// The hexagon of a column, in x = a · LAYER, y = b around its middle, is
// where |x| ≤ LAYER and |y ± LAYER · x| ≤ 3 / 4: the bisectors to its six
// neighbours, (±2, 0) and (±1, ±1) in (a, b).
const NORMALS = array<vec2f, 3>(vec2f(1.0, 0.0), vec2f(LAYER, 1.0), vec2f(-LAYER, 1.0));
const BOUNDS = vec3f(LAYER, 0.75, 0.75);

// Where `ray` enters a sample's run, a prism over its column's hexagon,
// in layers from the eye; −1 if it misses. `rates` are how fast the ray
// crosses each pair of the hexagon's sides, per layer.
fn meets(run: vec4f, rates: vec3f) -> f32 {
  // Where the ray is at layer c from the eye, relative to the middle: (−middle + slope · c) · (LAYER, 1).
  let q0 = -run.xy * vec2f(LAYER, 1.0);
  var lo = run.z;
  var hi = run.w;
  for (var s = 0; s < 3; s++) {
    let at = dot(NORMALS[s], q0);
    let t0 = (-BOUNDS[s] - at) / rates[s];
    let t1 = (BOUNDS[s] - at) / rates[s];
    lo = max(lo, min(t0, t1));
    hi = min(hi, max(t0, t1));
  }
  return select(-1.0, lo, lo < hi);
}

@fragment
fn resolve_fragment(in: Screen) -> @location(0) vec4f {
  let ray = ray_through(in.position.xy);
  let range = face_bands[ray.face];
  let last = vec2i(textureDimensions(sample_hits)) - 1;
  let base = vec2i(floor(in.position.xy / f32(factor) - 0.5));
  let qd = ray.slope * vec2f(LAYER, 1.0);
  // Never 0, so that a ray along a pair of sides divides to ±inf.
  let rates = vec3f(dot(NORMALS[0], qd), dot(NORMALS[1], qd), dot(NORMALS[2], qd)) + 1e-12;
  var best = vec4u(NONE);
  var at = 0.0;
  var nearest = 1e30;
  var sky_seen = false;
  var seen = vec4u(NONE);
  for (var s = 0; s < 4; s++) {
    let p = clamp(base + vec2i(s & 1, s >> 1), vec2i(0), last);
    let hit = textureLoad(sample_hits, p, 0);
    if hit.y == NONE {
      sky_seen = true;
      continue;
    }
    // Each texel once, and only the ray's face's.
    if any(seen == vec4u(hit.x)) || hit.y < range.x || hit.y >= range.x + range.y {
      continue;
    }
    seen[s] = hit.x;
    let c = meets(textureLoad(sample_runs, p, 0), rates);
    let distance = c * bitcast<f32>(hit.z);
    if c >= 0.0 && distance < nearest {
      nearest = distance;
      best = hit;
      at = c;
    }
  }
  if best.y == NONE && !sky_seen {
    // The nearest sample's.
    let p = clamp(vec2i(floor(in.position.xy / f32(factor))), vec2i(0), last);
    best = textureLoad(sample_hits, p, 0);
    at = textureLoad(sample_runs, p, 0).z;
  }
  if best.y == NONE {
    return vec4f(sky(globals.sky_view * ray.camera), 1.0);
  }
  let fog = 1.0 - exp(-at * bitcast<f32>(best.z) * LAYER / globals.fog_distance);
  return vec4f(mix(unpack4x8unorm(texels[best.x].x).rgb, globals.horizon.rgb, fog), 1.0);
}

// The picture, shown where egui's pass puts it.
@group(0) @binding(9) var picture: texture_2d<f32>;

@fragment
fn show_fragment(in: Screen) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(picture));
  return textureLoad(picture, vec2i(min(in.at * size, size - 1.0)), 0);
}
