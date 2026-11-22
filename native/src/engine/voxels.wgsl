// voxels.rs: rays through the world's own voxels, rhombic dodecahedra of the
// FCC lattice, from cell to cell across their faces, at a coarser grid the
// farther they go. One ray for every FACTOR × FACTOR pixels (sample_fragment);
// then each pixel tests its own ray against the voxels its four nearest
// samples met, and takes the nearest it enters (resolve_fragment).
// See-through voxels (a medium: fog, water, a far canopy) add up on the way,
// in the order the ray meets them, each by its path through it; pieces
// held in part (fcc's in_part) stop it whole or not at all, by a coin of
// where it enters them (`stops`).
// A voxel shows how it looks along the ray (`seen`), times the
// light on it: shaded, by the way its surface faces; lit, the light its tile
// found for it (world.rs), and its own glow, as the eye sees it (eye.rs:
// `shown`, adapted to what the samples meter). A brick no ray asked for yet,
// or, lit, not lit yet, is not on the GPU (voxels.rs), as one of a tile not
// made: rays take the next level, and ask for it (`asked`).
// voxels.rs puts the constants it shares with this file before it.

// The picture's size in pixels, and its sides over the focal length; the
// fog's distance, and what of the sky it hides; whether voxels are lit; the
// multiple of light the eye shows (eye.rs); the sky's light (linear); right,
// up and forward in the world; the light the eye shows white.
struct Globals {
  resolution: vec2f,
  extent: vec2f,
  fog: f32,
  sky_fog: f32,
  lit: u32,
  exposure: f32,
  sky_top: vec4f,
  horizon: vec4f,
  view: mat3x3f,
  white: f32,
}

// A grid level's window of WINDOW³ bricks: the site it starts at, in the
// level's units; where its table starts in `bricks`; its corner, in bricks
// of the level; how far from the eye (world units) rays read it; where its
// cells' counts start in `filled`; the box, in its sites, that what rays meet
// of it is in (voxels.rs's bounds). The table is a torus:
// a brick's entry is at its coordinates modulo WINDOW, so the window moves
// without moving the rest.
struct Level {
  min: vec3i,
  offset: u32,
  corner: vec3i,
  reach: f32,
  low: vec3f,
  filled: u32,
  high: vec3f,
}

// The entry of the brick `b` bricks from the window's corner.
fn entry(l: Level, b: vec3i) -> u32 {
  let w = (b + l.corner) & vec3i(WINDOW - 1);
  return l.offset + u32((w.z * WINDOW + w.y) * WINDOW + w.x);
}

// Whether no voxel meets cell `k` of scale `j` (voxels.rs's CELLS: k its
// corner, in blocks of 4^j bricks of the level) and no brick around it is unknown.
fn hollow(l: Level, k: vec3i, j: u32) -> bool {
  let side = WINDOW >> (2u * j);
  let w = k & vec3i(side - 1);
  return filled[l.filled + j * CELLS + u32(((w.z * side + w.y) * side + w.x) >> 1)] == 0u;
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> levels: array<Level>;
// Per level, its window's bricks' slots in `cells`, EMPTY or UNKNOWN (no tile yet: rays take the next level).
@group(0) @binding(2) var<storage, read> bricks: array<i32>;
// Per slot of the pool, its brick's 256 sites (Grid's order), each its look (fcc's Look), a word each.
@group(0) @binding(3) var<storage, read> cells: array<u32>;
// The eye in the world (grid 0's units).
@group(0) @binding(4) var<uniform> eye: vec4f;
// Per level, per cell of each scale in its window, how many bricks around it have voxels that meet it or are unknown (a torus, as `bricks`).
@group(0) @binding(5) var<storage, read> filled: array<u32>;
// Lit, per slot of the pool, the light on its brick's voxels as `cells` has them, LIGHT_WORDS a site: the light on
// it and what it glows, RGB9E5, 0 if none.
@group(0) @binding(6) var<storage, read> lights: array<u32>;
// Per slot of the pool, the next of its brick's (voxels.rs's Pool), EMPTY if none: a brick whose
// voxels look unlike through some faces has more (face_look), as `cells` has them.
@group(0) @binding(7) var<storage, read> next: array<i32>;
// A bit per entry of the levels' tables (as `bricks`): whether a ray took the next level at its brick, unknown, or read it for what it shows.
@group(0) @binding(11) var<storage, read_write> asked: array<atomic<u32>>;
// Lit, the samples by the stops of the luminance they meet (eye.rs's BINS).
@group(0) @binding(12) var<storage, read_write> metered: array<atomic<u32>, BINS>;

const NONE = 0xffffffffu;
// A ray's steps at a level at most: more than any needs. One that runs out
// is LOST, and shows magenta: a march must not stick, and if one does, it
// must not hide.
const STEPS = 1024;
const LOST = 0xfffffffeu;
// FACES (voxels.rs puts them before this file, fcc's VOXEL_FACES): a voxel
// is where n · (p − site) ≤ 1 for each, its neighbours at n.

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

// The sky's light along `ray` (linear).
fn sky_light(ray: vec3f) -> vec3f {
  let elevation = ray.y / length(ray.xz);
  return mix(globals.horizon.rgb, globals.sky_top.rgb, clamp(elevation / 0.625, 0.0, 1.0));
}

// The sky along `ray` as the eye sees it, fogged.
fn sky(ray: vec3f) -> vec3f {
  return mix(shown(sky_light(ray)), horizon(), globals.sky_fog);
}

// The horizon as the eye sees it: what the fog fades to.
fn horizon() -> vec3f {
  return shown(globals.horizon.rgb);
}

// The slot of the brick of `level` holding site `s`, EMPTY or UNKNOWN (also off the window).
fn brick_of(level: u32, s: vec3i) -> i32 {
  let l = levels[level];
  let b = (s - l.min) >> vec3u(BRICK_SHIFT);
  if any(b < vec3i(0)) || any(b >= vec3i(WINDOW)) {
    return UNKNOWN;
  }
  return bricks[entry(l, b)];
}

// Asks for the brick of `level` holding site `s`, if unknown in its window: for its tile, or its light.
fn ask(level: u32, s: vec3i) {
  let l = levels[level];
  let b = (s - l.min) >> vec3u(BRICK_SHIFT);
  if all(b >= vec3i(0)) && all(b < vec3i(WINDOW)) {
    let e = entry(l, b);
    if bricks[e] == UNKNOWN {
      atomicOr(&asked[e >> 5u], 1u << (e & 31u));
    }
  }
}

// The look of the voxel at site `s` of `level`, 0 if none, or not known yet: asked for then, as what shows is shaded by it.
fn voxel(level: u32, s: vec3i) -> u32 {
  ask(level, s);
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

// The look of the voxel at `r` in the brick at `slot`.
fn site_in(slot: i32, r: vec3i) -> u32 {
  return cells[u32(slot) * BRICK_SITES + site_index(r)];
}

// The light on a voxel, and what it glows (linear): it sends out its colour times the one, and the other.
struct Light {
  on: vec3f,
  glow: vec3f,
}

// Lit, the light of the voxel at `r` in the brick at `slot` (voxels.rs's `light`), words 0 if none.
fn light_in(slot: i32, r: vec3i) -> vec2u {
  let i = (u32(slot) * BRICK_SITES + site_index(r)) * LIGHT_WORDS;
  return vec2u(lights[i], lights[i + 1u]);
}

// Lit, the light of the voxel at site `s` of `level`, words 0 if none, or not known yet: asked for then.
fn light_at(level: u32, s: vec3i) -> vec2u {
  let slot = brick_of(level, s);
  if slot < 0 {
    ask(level, s);
    return vec2u(0u);
  }
  return light_in(slot, (s - levels[level].min) & vec3i(BRICK - 1));
}

fn light_of(words: vec2u) -> Light {
  return Light(rgb9e5(words.x), rgb9e5(words.y));
}

// The colour of the see-through voxel of look `v` at `r` in the brick at `slot`: under the light on it, shaded as its faces are on average.
fn medium_colour(slot: i32, r: vec3i, v: u32) -> vec3f {
  if globals.lit != 0u {
    return coloured(unpack4x8unorm(v).rgb, light_of(light_in(slot, r)));
  }
  return coloured(unpack4x8unorm(v).rgb, Light(vec3f(MEDIUM_SHADE), vec3f(0.0)));
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

// A ray along `d` as a rhombic dodecahedron's pairs of faces see it: per
// pair, which way it crosses them and the inverse of how fast, for the
// normals (1, 1, 0), (0, 1, 1), (1, 0, 1), then (1, −1, 0), (0, 1, −1),
// (1, 0, −1). Along a pair it does neither (0, and 1e30).
struct Ray {
  ways: array<vec3f, 2>,
  inverse: array<vec3f, 2>,
}

fn ray_of(d: vec3f) -> Ray {
  let rates = array<vec3f, 2>(d.xyz + d.yzx, d.xyz - d.yzx);
  var out: Ray;
  for (var k = 0; k < 2; k++) {
    out.ways[k] = select(sign(rates[k]), vec3f(0.0), abs(rates[k]) < vec3f(1e-9));
    out.inverse[k] = select(1.0 / rates[k], vec3f(1e30), abs(rates[k]) < vec3f(1e-9));
  }
  return out;
}

// Where `ray`, from `q` off the centre of a rhombic dodecahedron `r` times
// a voxel (a voxel if 1, a cell of voxels.rs's CELLS if more), comes
// between each pair of its faces and leaves it, per pair as Ray has them.
// A pair it runs along it is between always, or never.
struct Crossings {
  enter: array<vec3f, 2>,
  leave: array<vec3f, 2>,
}

fn crossings(ray: Ray, q: vec3f, r: f32) -> Crossings {
  let at = array<vec3f, 2>(q.xyz + q.yzx, q.xyz - q.yzx);
  var out: Crossings;
  for (var k = 0; k < 2; k++) {
    let along = ray.ways[k] == vec3f(0.0);
    let never = select(vec3f(-1e30), vec3f(1e30), abs(at[k]) > vec3f(r));
    out.enter[k] = select((-ray.ways[k] * r - at[k]) * ray.inverse[k], never, along);
    out.leave[k] = select((ray.ways[k] * r - at[k]) * ray.inverse[k], vec3f(1e30), along);
  }
  return out;
}

// Where the ray enters the dodecahedron, and where it leaves it.
fn entering(c: Crossings) -> f32 {
  let m = max(c.enter[0], c.enter[1]);
  return max(m.x, max(m.y, m.z));
}

fn leaving(c: Crossings) -> f32 {
  let m = min(c.leave[0], c.leave[1]);
  return min(m.x, min(m.y, m.z));
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
// times what each let through of them; and what they let through, each channel.
var<private> gathered = vec3f(0.0);
var<private> through = vec3f(1.0);
// What the see-through voxel of look `v` at `r` in the brick at `slot` stops of each channel of a ray crossing it:
// its alpha, unless the brick holds its voxels' own, in the slot after it (voxels.rs's STOPS).
fn medium_stops(slot: i32, r: vec3i, v: u32) -> vec3f {
  let more = next[slot];
  if more >= 0 && (more & STOPS) != 0 {
    return unpack4x8unorm(site_in(more & ~STOPS, r)).rgb;
  }
  return vec3f(f32(v >> 24u) / 255.0);
}
// What a see-through voxel that stops `stops` of a ray crossing it lets through along `path` of its grid's units, each channel.
fn lets_through(stops: vec3f, path: f32) -> vec3f {
  return pow(1.0 - stops, vec3f(path / CHORD));
}
// Where the ray from `o` leaves the cell of scale `j` (voxels.rs's CELLS)
// it is in at `mid`, if no voxel meets that cell: it meets none till then.
// Else 0, and till then, `held`: no need to look again.
fn across(l: Level, j: u32, ray: Ray, o: vec3f, d: vec3f, mid: f32, held: ptr<function, f32>) -> f32 {
  if mid <= *held {
    return 0.0;
  }
  let size = BRICK << (2u * j);
  let cell = site_at((o + d * mid - vec3f(l.min)) / f32(size) + vec3f(l.corner) * (f32(BRICK) / f32(size)));
  let out = leaving(crossings(ray, o - vec3f(l.min + cell * size - l.corner * BRICK), f32(size)));
  if !hollow(l, cell, j) {
    *held = out;
    return 0.0;
  }
  return out;
}

// Where the ray from `o` along `d` enters level `l`'s box (Level's) and leaves it.
fn within(l: Level, o: vec3f, d: vec3f) -> vec2f {
  var span = vec2f(0.0, 1e30);
  for (var k = 0; k < 3; k++) {
    if d[k] != 0.0 {
      let a = (l.low[k] - o[k]) / d[k];
      let b = (l.high[k] - o[k]) / d[k];
      span = vec2f(max(span.x, min(a, b)), min(span.y, max(a, b)));
    } else if o[k] < l.low[k] || o[k] > l.high[k] {
      span.y = -1.0;
    }
  }
  return span;
}

// Whether look `v` is of pieces held in part (fcc's in_part): under 255, its red odd.
fn in_part(v: u32) -> bool {
  return (v & 1u) == 1u && (v >> 24u) != 0u && (v >> 24u) != 255u;
}

// Whether pieces held in part of `alpha` in a voxel of `level` stop a ray entering it at `p` (the level's units): by
// a coin of the site of a lattice four times finer that it enters by, so as often as they stop rays, speckled
// finer than their voxel, where they are in the world.
fn stops(alpha: u32, level: u32, p: vec3f) -> bool {
  let h = bitcast<vec3u>(site_at(p * 4.0)) * vec3u(0x9e3779b1u, 0x85ebca77u, 0xc2b2ae3du);
  var x = h.x ^ h.y ^ h.z ^ (level * 0x27d4eb2fu);
  x = (x ^ (x >> 15u)) * 0x2c1b3c6du;
  return alpha > (x ^ (x >> 12u)) >> 24u;
}

fn march(d: vec3f) -> Hit {
  let ray = ray_of(d);
  var t = 0.0;
  let count = arrayLength(&levels);
  for (var level = 0u; level < count; level++) {
    let l = levels[level];
    if t >= l.reach {
      continue;
    }
    let scale = exp2(f32(level));
    let o = eye.xyz / scale;
    let end = l.reach / scale;
    // Only in the level's box: out of it, the ray meets nothing of the level.
    let span = within(l, o, d);
    let stop = min(end, span.y);
    var at = min(max(t / scale, span.x), stop);
    var s = site_at(o + d * at);
    var brick = vec3i(-1 << 30);
    var slot = UNKNOWN;
    var medium = vec3f(0.0);
    var tint = vec3f(0.0);
    // Of the cells of the two scales, fine and coarse, till where the ray is in one some voxel meets.
    var fine = -1.0;
    var coarse = -1.0;
    var steps = 0;
    for (; steps < STEPS && at < stop; steps++) {
      let b = (s - l.min) >> vec3u(BRICK_SHIFT);
      if any(b != brick) {
        brick = b;
        slot = brick_of(level, s);
      }
      // No tile here yet, not lit yet, or off the window: the next coarser level from here on, asking for it.
      if slot == UNKNOWN {
        ask(level, s);
        break;
      }
      if slot >= 0 {
        let r = (s - l.min) & vec3i(BRICK - 1);
        let v = site_in(slot, r);
        let alpha = v >> 24u;
        // Opaque, or pieces held in part that stop it, unless the eye is in it.
        let part = in_part(v);
        let stopped = alpha == 255u || part && stops(alpha, level, o + d * at);
        if stopped && (steps > 0 || at > 0.0) {
          return Hit(level, s, v, at * scale);
        }
        if alpha != 0u && !part {
          medium = medium_stops(slot, r, v);
          tint = medium_colour(slot, r, v);
        }
      }
      // Out across the nearest face it leaves by, into the neighbour there.
      let c = crossings(ray, o - vec3f(s), 1.0);
      let plus = c.leave[0];
      let minus = c.leave[1];
      let leave = leaving(c);
      // The pair (k, k + 1 of x, y, z) it crosses, and whether by + or −.
      let by_plus = min(plus.x, min(plus.y, plus.z)) <= min(minus.x, min(minus.y, minus.z));
      let side = select(minus, plus, by_plus);
      let k = select(select(2, 1, side.y <= side.z), 0, side.x <= side.y && side.x <= side.z);
      let way = select(ray.ways[1], ray.ways[0], by_plus)[k];
      var step = vec3i(0);
      step[k] = i32(way);
      step[(k + 1) % 3] = i32(select(-way, way, by_plus));
      if any(medium != vec3f(0.0)) {
        // Its path through the voxel: from where it entered (or the eye) to where it leaves.
        let stops = 1.0 - lets_through(medium, leave - at);
        let fog = 1.0 - exp(-(at + leave) * 0.5 * scale / globals.fog);
        gathered += through * stops * mix(tint, horizon(), fog);
        through *= 1.0 - stops;
        medium = vec3f(0.0);
        if all(through < vec3f(OPAQUE)) {
          return Hit(NONE, vec3i(0), 0u, 0.0);
        }
      } else {
        // The cells around the middle of the ray's way through this empty voxel, coarse first:
        // if no voxel meets one, the ray meets none till it leaves it, so on across it, as across a voxel.
        // Landed on a face, `s` may be the voxel behind, left already: then on into the next.
        if leave > at {
          let mid = (at + leave) * 0.5;
          var out = across(l, 1u, ray, o, d, mid, &coarse);
          if out <= leave {
            out = across(l, 0u, ray, o, d, mid, &fine);
          }
          if out > leave {
            // No farther than the level is read: a cell at its window's edge reaches past it (voxels.rs's CELLS).
            at = min(out, end);
            s = site_at(o + d * at);
            continue;
          }
        }
      }
      at = max(at, leave);
      s += step;
    }
    if steps == STEPS {
      return Hit(LOST, vec3i(0), 0u, 0.0);
    }
    // Out of the box, on to the next level from where it would have taken it.
    if at >= stop {
      at = max(at, end);
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

fn rgb9e5(w: u32) -> vec3f {
  return vec3f(vec3u(w, w >> 9u, w >> 18u) & vec3u(511u)) * exp2(f32(w >> 27u) - 24.0);
}

fn to_linear(c: vec3f) -> vec3f {
  return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

fn to_srgb(c: vec3f) -> vec3f {
  return select(1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3f(0.0031308));
}

// Colour `rgb` (sRGB) under `light`: shaded, times a shade, as the splats paint it; lit, the light it sends
// as the eye sees it.
fn coloured(rgb: vec3f, light: Light) -> vec3f {
  if globals.lit == 0u {
    return rgb * light.on;
  }
  return shown(sent(rgb, light));
}

// The light colour `rgb` (sRGB) sends under `light` (linear): times the light on it, and its glow, as light's
// Light::relight has it.
fn sent(rgb: vec3f, light: Light) -> vec3f {
  return to_linear(rgb) * light.on + light.glow;
}

const LUMINANCE = vec3f(0.2126, 0.7152, 0.0722);

// Light `c` (linear) as the eye sees it (eye.rs), sRGB: each channel times the exposure, x, as Reinhard's curve
// takes it, x (1 + x / w²) / (1 + x), w white: about x where dark, 1 at w, past it white; w 1, the light as it is.
fn shown(c: vec3f) -> vec3f {
  let x = c * globals.exposure;
  return to_srgb(min(x * (1.0 + x / (globals.white * globals.white)) / (1.0 + x), vec3f(1.0)));
}

// Lit, counts light `c` (linear) in the meter's bin of the stops of its luminance (eye.rs).
fn meter(c: vec3f) {
  if globals.lit != 0u {
    let stops = (log2(max(dot(c, LUMINANCE), 1e-30)) - LOWEST) * f32(BINS) / SPAN;
    atomicAdd(&metered[u32(clamp(stops, 0.0, f32(BINS - 1u)))], 1u);
  }
}

// The look through face `k` (of FACES) of the voxel hit: its own, unless its brick's next slots hold what its
// voxels' faces see (voxels.rs's `sights`): which sight each face sees, two bits a face (0: its own), then the
// sights, one a slot.
fn face_look(hit: Hit, k: u32) -> u32 {
  var sights = next[brick_of(hit.level, hit.site)];
  // Past what its see-through voxels stop, if it holds that.
  if sights >= 0 && (sights & STOPS) != 0 {
    sights = next[sights & ~STOPS];
  }
  if sights < 0 {
    return hit.voxel;
  }
  let r = (hit.site - levels[hit.level].min) & vec3i(BRICK - 1);
  let which = (site_in(sights, r) >> (2u * k)) & 3u;
  if which == 0u {
    return hit.voxel;
  }
  for (var j = 0u; j < which; j++) {
    sights = next[sights];
  }
  return site_in(sights, r);
}

// What the voxel hit looks like along `d` (sRGB): the mean (linear) of its looks through the faces turned
// most against `d`, alike (fcc's `facing`), wherever the ray enters it.
fn seen(hit: Hit, d: vec3f) -> vec3f {
  var least = 1e30;
  for (var k = 0u; k < 12u; k++) {
    least = min(least, dot(FACES[k], d));
  }
  var sum = vec3f(0.0);
  var faces = 0.0;
  for (var k = 0u; k < 12u; k++) {
    if dot(FACES[k], d) <= least + 1e-6 {
      sum += to_linear(unpack4x8unorm(face_look(hit, k)).rgb);
      faces += 1.0;
    }
  }
  return to_srgb(sum / faces);
}

// The light on the voxel hit: shaded, the mean of its open faces' shades; lit, its tile's (lit).
fn light_on(hit: Hit) -> Light {
  if globals.lit != 0u {
    return lit(hit);
  }
  var shade = 0.0;
  var open = 0.0;
  for (var k = 0; k < 12; k++) {
    if (voxel(hit.level, hit.site + vec3i(FACES[k])) >> 24u) < 255u {
      shade += face_shade(FACES[k]);
      open += 1.0;
    }
  }
  return Light(vec3f(select(1.0, shade / open, open > 0.0)), vec3f(0.0));
}

// Lit, the light on the voxel hit; one with none (met where levels meet, inside what is solid) takes the mean of its neighbours' that have, if any.
fn lit(hit: Hit) -> Light {
  let own = light_at(hit.level, hit.site);
  if any(own != vec2u(0u)) {
    return light_of(own);
  }
  var sum = Light(vec3f(0.0), vec3f(0.0));
  var count = 0.0;
  for (var k = 0; k < 12; k++) {
    let light = light_at(hit.level, hit.site + vec3i(FACES[k]));
    if any(light != vec2u(0u)) {
      sum.on += rgb9e5(light.x);
      sum.glow += rgb9e5(light.y);
      count += 1.0;
    }
  }
  return Light(sum.on / max(count, 1.0), sum.glow / max(count, 1.0));
}

// What each sample met: its site, and level | voxel << 8 (NONE if
// nothing); and the voxel's colour, and what the see-through voxels in front gave, and what they let through.
@group(0) @binding(8) var sample_hits: texture_2d<u32>;
@group(0) @binding(9) var sample_colours: texture_2d<u32>;

struct Sample {
  @location(0) hit: vec4u,
  @location(1) colour: vec4u,
}

@fragment
fn sample_fragment(in: Screen) -> Sample {
  let p = (floor(in.position.xy) + 0.5) * f32(FACTOR);
  let d = ray_at(p);
  let hit = march(d);
  let medium = vec2u(pack4x8unorm(vec4f(gathered, 1.0)), pack4x8unorm(vec4f(through, 1.0)));
  if hit.level >= LOST {
    meter(sky_light(d));
    return Sample(vec4u(0u, 0u, 0u, hit.level), vec4u(0u, medium, 0u));
  }
  let look = seen(hit, d);
  let light = light_on(hit);
  meter(sent(look, light));
  let colour = coloured(look, light);
  return Sample(vec4u(bitcast<vec3u>(hit.site), hit.level), vec4u(pack4x8unorm(vec4f(colour, 1.0)), medium, 0u));
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

// A sample's hit, for a pixel whose ray, along `d`, enters its voxel, unless pieces held in part let that ray by.
fn consider(c: Choice, hit: vec4u, k: i32, ray: Ray, d: vec3f) -> Choice {
  var out = c;
  if hit.w == NONE {
    out.sky_seen = true;
    return out;
  }
  let level = hit.w & 255u;
  let site = bitcast<vec3i>(hit.xyz);
  let scale = exp2(f32(level));
  let crossed = crossings(ray, eye.xyz / scale - vec3f(site), 1.0);
  let enter = entering(crossed);
  let leave = leaving(crossed);
  let t = enter * scale;
  let v = in_brick(level, brick_of(level, site), site);
  if in_part(v) && !stops(v >> 24u, level, eye.xyz / scale + d * enter) {
    return out;
  }
  if enter < leave && leave > 0.0 && t < c.nearest {
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
  let ray = ray_of(d);
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
  if any(vec4u(h0.w, h1.w, h2.w, h3.w) == vec4u(LOST)) {
    return vec4f(1.0, 0.0, 1.0, 1.0);
  }
  var c = Choice(-1, 1e30, false);
  c = consider(c, h0, 0, ray, d);
  c = consider(c, h1, 1, ray, d);
  c = consider(c, h2, 2, ray, d);
  c = consider(c, h3, 3, ray, d);
  // What the samples agreeing with the pixel saw (w NONE: nothing), or the only one.
  var seen = vec4u(0u, 0u, 0u, NONE);
  var only = -1;
  var best = c.best;
  if best < 0 && !c.sky_seen {
    // None covers it: its nearest sample's voxel, where its ray would enter it.
    let near = vec2i(floor(position / f32(FACTOR))) - base;
    only = near.x | (near.y << 1u);
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
      nearest = max(entering(crossings(ray, eye.xyz / scale - vec3f(bitcast<vec3i>(seen.xyz)), 1.0)), 0.0) * scale;
    }
    behind = mix(unpack4x8unorm(colours).rgb, horizon(), 1.0 - exp(-nearest / globals.fog));
  }
  let g = vec2f(1.0) - f;
  let w = vec4f(
    agreeing(h0, 0, g.x * g.y, seen, only),
    agreeing(h1, 1, f.x * g.y, seen, only),
    agreeing(h2, 2, g.x * f.y, seen, only),
    agreeing(h3, 3, f.x * f.y, seen, only),
  );
  let m0 = textureLoad(sample_colours, p0, 0).yz;
  let m1 = textureLoad(sample_colours, p1, 0).yz;
  let m2 = textureLoad(sample_colours, p2, 0).yz;
  let m3 = textureLoad(sample_colours, p3, 0).yz;
  let gave = w.x * unpack4x8unorm(m0.x) + w.y * unpack4x8unorm(m1.x) + w.z * unpack4x8unorm(m2.x) + w.w * unpack4x8unorm(m3.x);
  let let_through = w.x * unpack4x8unorm(m0.y) + w.y * unpack4x8unorm(m1.y) + w.z * unpack4x8unorm(m2.y) + w.w * unpack4x8unorm(m3.y);
  return vec4f((gave.rgb + let_through.rgb * behind) / dot(w, vec4f(1.0)), 1.0);
}

// The picture, shown where egui's pass puts it.
@group(0) @binding(10) var picture: texture_2d<f32>;

@fragment
fn show_fragment(in: Screen) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(picture));
  return textureLoad(picture, vec2i(min(in.at * size, size - 1.0)), 0);
}
