// march.rs: rays through the world's own voxels, rhombic dodecahedra of the
// FCC lattice, from cell to cell across their faces, at a coarser grid the
// farther they go. One ray for every FACTOR × FACTOR pixels (sample_fragment);
// then each pixel tests its own ray against the voxels its four nearest
// samples met, and takes the nearest it enters (resolve_fragment).
// See-through voxels (a medium: fog, water, a far canopy) add up on the way,
// in the order the ray meets them, each by its path through it.
// Lit, a voxel shows the light its tile found for it (world.rs), not its
// material's colour shaded by the way its faces face, once its tile is lit.
// march.rs puts the constants it shares with this file before it.

// The picture's size in pixels, and its sides over the focal length; the
// fog's distance, and what of the sky it hides; whether voxels are lit; the
// sky's colours; right, up and forward in the world.
struct Globals {
  resolution: vec2f,
  extent: vec2f,
  fog: f32,
  sky_fog: f32,
  lit: u32,
  sky_top: vec4f,
  horizon: vec4f,
  view: mat3x3f,
}

// A grid level's window of WINDOW³ bricks: the site it starts at, in the
// level's units; where its table starts in `bricks`; its corner, in bricks
// of the level; how far from the eye (world units) rays read it; where its
// block map starts in `blocks`. The table is a torus: a brick's entry is at
// its coordinates modulo WINDOW, so the window moves without moving the rest.
struct Level {
  min: vec3i,
  offset: u32,
  corner: vec3i,
  reach: f32,
  blocks: u32,
}

// The entries of the brick `b` bricks from the window's corner, and of its block.
fn entry(l: Level, b: vec3i) -> u32 {
  let w = (b + l.corner) & vec3i(WINDOW - 1);
  return l.offset + u32((w.z * WINDOW + w.y) * WINDOW + w.x);
}

fn block_entry(l: Level, b: vec3i) -> u32 {
  let w = ((b + l.corner) & vec3i(WINDOW - 1)) >> vec3u(BLOCK_SHIFT);
  let across = WINDOW / BLOCK;
  return l.blocks + u32((w.z * across + w.y) * across + w.x);
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> levels: array<Level>;
// Per level, its window's bricks' slots in `cells`, EMPTY or UNKNOWN (no tile yet: rays take the next level).
@group(0) @binding(2) var<storage, read> bricks: array<i32>;
// Per slot of the pool, its brick's 256 sites (Grid's order), each material | alpha << 8, two a word.
@group(0) @binding(3) var<storage, read> cells: array<u32>;
// Per material, 0xrrggbb.
@group(0) @binding(4) var<storage, read> palette: array<u32>;
// The eye in the world (grid 0's units).
@group(0) @binding(5) var<uniform> eye: vec4f;
// Per level, how many bricks of each block of its window are not EMPTY (a torus, as `bricks`).
@group(0) @binding(6) var<storage, read> blocks: array<u32>;
// Lit, per slot of the pool, its brick's sites' light as `cells` has them, one a word: sRGB, 0 if none.
@group(0) @binding(7) var<storage, read> lights: array<u32>;

const NONE = 0xffffffffu;
// Face pairs' normals: a voxel is where |n · (p − site)| ≤ 1 for each, its
// neighbours at ±n.
const NORMALS = array<vec3f, 6>(vec3f(1.0, 1.0, 0.0), vec3f(1.0, -1.0, 0.0), vec3f(0.0, 1.0, 1.0), vec3f(0.0, 1.0, -1.0), vec3f(1.0, 0.0, 1.0), vec3f(1.0, 0.0, -1.0));

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

// The ray through pixel `p`, in the world, of length 1.
fn ray_at(p: vec2f) -> vec3f {
  let s = p / globals.resolution - 0.5;
  return normalize(globals.view * vec3f(vec2f(s.x, -s.y) * globals.extent, 1.0));
}

fn sky(ray: vec3f) -> vec3f {
  let elevation = ray.y / length(ray.xz);
  let rgb = mix(globals.horizon.rgb, globals.sky_top.rgb, clamp(elevation / 0.625, 0.0, 1.0));
  return mix(rgb, globals.horizon.rgb, globals.sky_fog);
}

// The slot of the brick of `level` holding site `s`: −1 if it is empty, −2 if outside.
fn brick_of(level: u32, s: vec3i) -> i32 {
  let l = levels[level];
  let b = (s - l.min) >> vec3u(BRICK_SHIFT);
  if any(b < vec3i(0)) || any(b >= vec3i(WINDOW)) {
    return -2;
  }
  return bricks[entry(l, b)];
}

// The voxel at site `s` of `level`: material | alpha << 8, 0 if none.
fn voxel(level: u32, s: vec3i) -> u32 {
  return in_brick(level, brick_of(level, s), s);
}

// The voxel at site `s` of `level` in the brick at `slot`.
fn in_brick(level: u32, slot: i32, s: vec3i) -> u32 {
  if slot < 0 {
    return 0u;
  }
  return site_in(slot, (s - levels[level].min) & vec3i(BRICK - 1));
}

// Where site `r` of a brick is among its sites (Grid's order: z, y, then every other x).
fn site_index(r: vec3i) -> u32 {
  return u32(((r.z * BRICK + r.y) * BRICK + r.x) >> 1u);
}

// The voxel at `r` in the brick at `slot`, two a word.
fn site_in(slot: i32, r: vec3i) -> u32 {
  let i = site_index(r);
  return (cells[u32(slot) * BRICK_WORDS + (i >> 1u)] >> ((i & 1u) * 16u)) & 0xffffu;
}

// Lit, the light of the voxel at `r` in the brick at `slot`, 0 if none.
fn light_in(slot: i32, r: vec3i) -> u32 {
  return lights[u32(slot) * BRICK_SITES + site_index(r)];
}

// Lit, the light of the voxel at site `s` of `level`, 0 if none.
fn light_at(level: u32, s: vec3i) -> u32 {
  let slot = brick_of(level, s);
  if slot < 0 {
    return 0u;
  }
  return light_in(slot, (s - levels[level].min) & vec3i(BRICK - 1));
}

// The colour of the see-through voxel `v` at `r` in the brick at `slot`: lit, its light; else (or till its tile is lit) its material's, shaded as its faces are on average.
fn medium_colour(slot: i32, r: vec3i, v: u32) -> vec3f {
  if globals.lit != 0u {
    let light = light_in(slot, r);
    if light != 0u {
      return unpack4x8unorm(light).rgb;
    }
  }
  return rgb(v & 255u) * MEDIUM_SHADE;
}

// The site of the FCC lattice (x + y + z even) whose voxel holds `p`.
fn site_at(p: vec3f) -> vec3i {
  var r = round(p);
  if ((i32(r.x) + i32(r.y) + i32(r.z)) & 1) != 0 {
    let e = p - r;
    let a = abs(e);
    let k = select(select(2, 1, a.y >= a.z), 0, a.x >= a.y && a.x >= a.z);
    r[k] += select(-1.0, 1.0, e[k] >= 0.0);
  }
  return vec3i(r);
}

// Where a ray from `o` along `d` (in a grid's units) enters and leaves the voxel of site `s`.
struct Span {
  enter: f32,
  leave: f32,
}

fn span(o: vec3f, d: vec3f, s: vec3i) -> Span {
  let q = o - vec3f(s);
  var out = Span(-1e30, 1e30);
  for (var k = 0; k < 6; k++) {
    let n = NORMALS[k];
    let at = dot(n, q);
    let rate = dot(n, d);
    if abs(rate) < 1e-9 {
      if abs(at) > 1.0 {
        out.leave = -1e30;
      }
      continue;
    }
    let t0 = (-1.0 - at) / rate;
    let t1 = (1.0 - at) / rate;
    out.enter = max(out.enter, min(t0, t1));
    out.leave = min(out.leave, max(t0, t1));
  }
  return out;
}

// What a ray met first: its level, site and voxel, how far (world units).
struct Hit {
  level: u32,
  site: vec3i,
  voxel: u32,
  t: f32,
}

// A voxel's mean chord in its grid's units (worldgen's CHORD): a site's alpha
// is what it stops of rays crossing it, along that.
const CHORD = 0.9428090;
// The mean of a voxel's faces' shades (face_shade): a medium's.
const MEDIUM_SHADE = 0.7666667;
// Less let through than this, and nothing behind shows.
const OPAQUE = 1.0 / 256.0;
// What the see-through voxels in front of the ray's hit gave, their colours
// times what each let through of them; and what they let through.
var<private> gathered = vec3f(0.0);
var<private> through = 1.0;

fn march(d: vec3f) -> Hit {
  // Per pair of faces, how fast the ray crosses it, which way, and its inverse:
  // the normals (1, 1, 0), (0, 1, 1), (1, 0, 1), then (1, −1, 0), (0, 1, −1), (1, 0, −1).
  let rates = array<vec3f, 2>(d.xyz + d.yzx, d.xyz - d.yzx);
  let ways = array<vec3f, 2>(
    select(sign(rates[0]), vec3f(0.0), abs(rates[0]) < vec3f(1e-9)),
    select(sign(rates[1]), vec3f(0.0), abs(rates[1]) < vec3f(1e-9)),
  );
  let inverse = array<vec3f, 2>(
    select(1.0 / rates[0], vec3f(1e30), abs(rates[0]) < vec3f(1e-9)),
    select(1.0 / rates[1], vec3f(1e30), abs(rates[1]) < vec3f(1e-9)),
  );
  var t = 0.0;
  var first = true;
  let count = arrayLength(&levels);
  for (var level = 0u; level < count; level++) {
    let l = levels[level];
    if t >= l.reach {
      continue;
    }
    let scale = exp2(f32(level));
    let o = eye.xyz / scale;
    let end = l.reach / scale;
    var at = t / scale;
    var s = site_at(o + d * at);
    var brick = vec3i(-1 << 30);
    var slot = -2;
    var medium = 0u;
    var tint = vec3f(0.0);
    for (var steps = 0; steps < 1024 && at < end; steps++) {
      let b = (s - l.min) >> vec3u(BRICK_SHIFT);
      if any(b != brick) {
        brick = b;
        let inside = all(b >= vec3i(0)) && all(b < vec3i(WINDOW));
        slot = select(-2, bricks[entry(l, b)], inside);
        // Off the level's window and going away from it: nothing more at this level.
        if !inside && any((b < vec3i(0) & d <= vec3f(0.0)) | (b >= vec3i(WINDOW) & d >= vec3f(0.0))) {
          break;
        }
      }
      // No tile here yet: the next coarser level from here on.
      if slot == UNKNOWN {
        break;
      }
      if slot < 0 {
        // An empty brick, or block of them: on to where the ray leaves its sites' voxels' middles.
        // Blocks are the level's, aligned to its bricks, not to the window: its first brick, from the window's corner.
        let block = ((b + l.corner) >> vec3u(BLOCK_SHIFT)) * BLOCK - l.corner;
        let whole = slot == EMPTY && blocks[block_entry(l, b)] == 0u;
        let size = f32(select(BRICK, BRICK * BLOCK, whole));
        let lo = vec3f(select(b, block, whole) * BRICK + l.min) - 0.5;
        let exits = select((select(lo, lo + size, d > vec3f(0.0)) - o) / d, vec3f(1e30), d == vec3f(0.0));
        let out = min(min(exits.x, exits.y), exits.z);
        if out > at + 1e-3 {
          at = out + 1e-3;
          s = site_at(o + d * at);
          first = false;
          continue;
        }
      } else {
        let r = (s - l.min) & vec3i(BRICK - 1);
        let v = site_in(slot, r);
        if v >> 8u == 255u && !first {
          return Hit(level, s, v, at * scale);
        }
        medium = v;
        if v != 0u {
          tint = medium_colour(slot, r, v);
        }
      }
      first = false;
      // Out across the nearest face it leaves by, into the neighbour there.
      let q = o - vec3f(s);
      // A pair the ray runs along it never leaves by.
      let plus = select((ways[0] - (q.xyz + q.yzx)) * inverse[0], vec3f(1e30), ways[0] == vec3f(0.0));
      let minus = select((ways[1] - (q.xyz - q.yzx)) * inverse[1], vec3f(1e30), ways[1] == vec3f(0.0));
      let least = min(plus, minus);
      let leave = min(least.x, min(least.y, least.z));
      // The pair (k, k + 1 of x, y, z) it crosses, and whether by + or −.
      let by_plus = min(plus.x, min(plus.y, plus.z)) <= min(minus.x, min(minus.y, minus.z));
      let side = select(minus, plus, by_plus);
      let k = select(select(2, 1, side.y <= side.z), 0, side.x <= side.y && side.x <= side.z);
      let way = select(ways[1], ways[0], by_plus)[k];
      var step = vec3i(0);
      step[k] = i32(way);
      step[(k + 1) % 3] = i32(select(-way, way, by_plus));
      if medium != 0u {
        // Its path through the voxel: from where it entered (or the eye) to where it leaves.
        let stops = 1.0 - pow(1.0 - f32(medium >> 8u) / 255.0, (leave - at) / CHORD);
        let fog = 1.0 - exp(-(at + leave) * 0.5 * scale / globals.fog);
        gathered += through * stops * mix(tint, globals.horizon.rgb, fog);
        through *= 1.0 - stops;
        medium = 0u;
        if through < OPAQUE {
          return Hit(NONE, vec3i(0), 0u, 0.0);
        }
      }
      at = max(at, leave);
      s += step;
    }
    t = max(t, at * scale);
  }
  return Hit(NONE, vec3i(0), 0u, 0.0);
}

// The shade of a face by its outward normal in the world (tables.rs's face_shade).
fn face_shade(n: vec3f) -> f32 {
  let sides = select(vec3f(0.8, 1.0, 0.65), vec3f(0.7, 0.55, 0.9), n < vec3f(0.0));
  return dot(n * n, sides) / dot(n, n);
}

fn rgb(material: u32) -> vec3f {
  let c = palette[material];
  return vec3f(f32((c >> 16u) & 255u), f32((c >> 8u) & 255u), f32(c & 255u)) / 255.0;
}

// As the splats paint a voxel unlit: its material's colour times the mean of its open faces' shades.
fn shaded(hit: Hit) -> vec3f {
  var shade = 0.0;
  var open = 0.0;
  for (var k = 0; k < 12; k++) {
    let n = NORMALS[k % 6] * select(1.0, -1.0, k >= 6);
    if (voxel(hit.level, hit.site + vec3i(n)) >> 8u) < 255u {
      shade += face_shade(n);
      open += 1.0;
    }
  }
  return rgb(hit.voxel & 255u) * select(1.0, shade / open, open > 0.0);
}

// Lit, the light of the voxel hit; one with none (met where levels meet, inside what is solid) takes the mean of its neighbours' that have;
// if none has (its tile not lit yet), it shows shaded, as unlit.
fn lit(hit: Hit) -> vec3f {
  let own = light_at(hit.level, hit.site);
  if own != 0u {
    return unpack4x8unorm(own).rgb;
  }
  var sum = vec3f(0.0);
  var count = 0.0;
  for (var k = 0; k < 12; k++) {
    let n = NORMALS[k % 6] * select(1.0, -1.0, k >= 6);
    let light = light_at(hit.level, hit.site + vec3i(n));
    if light != 0u {
      sum += unpack4x8unorm(light).rgb;
      count += 1.0;
    }
  }
  if count == 0.0 {
    return shaded(hit);
  }
  return sum / count;
}

// What each sample met: its site, and level | voxel << 8 (NONE if
// nothing); and the voxel's colour, the mean of its open faces' shades,
// and what the see-through voxels in front gave (rgb) and let through (a).
@group(0) @binding(8) var sample_hits: texture_2d<u32>;
@group(0) @binding(9) var sample_colours: texture_2d<u32>;

struct Sample {
  @location(0) hit: vec4u,
  @location(1) colour: vec2u,
}

@fragment
fn sample_fragment(in: Screen) -> Sample {
  let p = (floor(in.position.xy) + 0.5) * f32(FACTOR);
  let hit = march(ray_at(p));
  let medium = pack4x8unorm(vec4f(gathered, through));
  if hit.level == NONE {
    return Sample(vec4u(0u, 0u, 0u, NONE), vec2u(0u, medium));
  }
  var colour: vec3f;
  if globals.lit != 0u {
    colour = lit(hit);
  } else {
    colour = shaded(hit);
  }
  return Sample(vec4u(bitcast<vec3u>(hit.site), hit.level | hit.voxel << 8u), vec2u(pack4x8unorm(vec4f(colour, 1.0)), medium));
}

// Each pixel: the nearest voxel its own ray enters of those its four nearest
// samples met (else the sky they saw, else its nearest sample's voxel), behind
// the medium in front of it; the medium between those samples' that saw the
// same, as each sample's medium is what lies in front of what it saw.
// The four samples are taken once each, unrolled: indexing them costs more.

// The nearest voxel a pixel's ray enters so far: which sample met it, how far; and whether any saw nothing.
struct Choice {
  best: i32,
  nearest: f32,
  sky_seen: bool,
}

fn consider(c: Choice, hit: vec4u, k: i32, d: vec3f) -> Choice {
  var out = c;
  if hit.w == NONE {
    out.sky_seen = true;
    return out;
  }
  let scale = exp2(f32(hit.w & 255u));
  let s = span(eye.xyz / scale, d, bitcast<vec3i>(hit.xyz));
  let t = s.enter * scale;
  if s.enter < s.leave && s.leave > 0.0 && t < c.nearest {
    out = Choice(k, t, c.sky_seen);
  }
  return out;
}

// Sample `k`'s weight `w` if it agrees with the pixel (saw `seen`, or is the `only` one), else 0.
fn agreeing(hit: vec4u, k: i32, w: f32, seen: vec4u, only: i32) -> f32 {
  return select(0.0, max(w, 1e-4), select(all(hit == seen), k == only, only >= 0));
}

@fragment
fn resolve_fragment(in: Screen) -> @location(0) vec4f {
  let position = in.position.xy;
  let d = ray_at(position);
  let last = vec2i(textureDimensions(sample_hits)) - 1;
  let at = position / f32(FACTOR) - 0.5;
  let base = vec2i(floor(at));
  let f = at - floor(at);
  let p0 = clamp(base, vec2i(0), last);
  let p1 = clamp(base + vec2i(1, 0), vec2i(0), last);
  let p2 = clamp(base + vec2i(0, 1), vec2i(0), last);
  let p3 = clamp(base + vec2i(1, 1), vec2i(0), last);
  let h0 = textureLoad(sample_hits, p0, 0);
  let h1 = textureLoad(sample_hits, p1, 0);
  let h2 = textureLoad(sample_hits, p2, 0);
  let h3 = textureLoad(sample_hits, p3, 0);
  var c = Choice(-1, 1e30, false);
  c = consider(c, h0, 0, d);
  c = consider(c, h1, 1, d);
  c = consider(c, h2, 2, d);
  c = consider(c, h3, 3, d);
  // What the samples agreeing with the pixel saw (w NONE: nothing), or the only one.
  var seen = vec4u(0u, 0u, 0u, NONE);
  var only = -1;
  var best = c.best;
  if best < 0 && !c.sky_seen {
    // None covers it: its nearest sample's voxel, where its ray would enter it.
    let near = vec2i(floor(position / f32(FACTOR))) - base;
    only = near.x | near.y << 1u;
    best = only;
  }
  let p = clamp(base + vec2i(best & 1, best >> 1), vec2i(0), last);
  let colours = textureLoad(sample_colours, p, 0).x;
  var behind = sky(d);
  if best >= 0 {
    seen = select(select(h0, h1, best == 1), select(h2, h3, best == 3), best >= 2);
    var nearest = c.nearest;
    if only >= 0 {
      let scale = exp2(f32(seen.w & 255u));
      nearest = max(span(eye.xyz / scale, d, bitcast<vec3i>(seen.xyz)).enter, 0.0) * scale;
    }
    behind = mix(unpack4x8unorm(colours).rgb, globals.horizon.rgb, 1.0 - exp(-nearest / globals.fog));
  }
  let g = vec2f(1.0) - f;
  let w = vec4f(
    agreeing(h0, 0, g.x * g.y, seen, only),
    agreeing(h1, 1, f.x * g.y, seen, only),
    agreeing(h2, 2, g.x * f.y, seen, only),
    agreeing(h3, 3, f.x * f.y, seen, only),
  );
  let medium = (w.x * unpack4x8unorm(textureLoad(sample_colours, p0, 0).y) + w.y * unpack4x8unorm(textureLoad(sample_colours, p1, 0).y)
    + w.z * unpack4x8unorm(textureLoad(sample_colours, p2, 0).y) + w.w * unpack4x8unorm(textureLoad(sample_colours, p3, 0).y)) / dot(w, vec4f(1.0));
  return vec4f(medium.rgb + medium.a * behind, 1.0);
}

// The picture, shown where egui's pass puts it.
@group(0) @binding(10) var picture: texture_2d<f32>;

@fragment
fn show_fragment(in: Screen) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(picture));
  return textureLoad(picture, vec2i(min(in.at * size, size - 1.0)), 0);
}
