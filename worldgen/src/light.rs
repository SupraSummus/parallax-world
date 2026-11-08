//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (bake's light.rs).
//!
//! Light comes to a site from its 12 neighbours' directions: from the
//! neighbour one step along each, and past it from the next coarser
//! level's light. The neighbour sends what it glows, and of what it
//! stops (its alpha) its colour times the light on it, also from the
//! coarser level; the rest of what comes from past it gets through. So
//! each level's light comes from coarser ones only, the coarsest's from
//! the sky.
//!
//! The grids say how light sees each site, whether a voxel is shown there
//! or not (VoxelGrid's `alpha`, `seen`, `glow`): a coarse site a small
//! lamp is in stops little, and glows as bright as the lamp, on average
//! over its faces.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use bake::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use bake::slicing::GRID_LEVELS;

use crate::World;
use crate::generator::{Generator, generate, materials_of};
use crate::grid::{ALPHA, Box3, GLOW_ONE, VoxelGrid};
use crate::picture::{FACES, Picture, nearest_site};

type Rgb = [f32; 3];

const N: usize = 12;

/** The directions light comes along: towards the neighbours, through the voxel's faces. */
const DIRECTIONS: [[i64; 3]; N] = FACES;

/** At the coarsest level, steps along a direction before the sky's light takes over; at the others, one before the coarser level's does. */
const TOP_STEPS: usize = 64;
/** Sites per tile in x and z, at each level, as native's; a tile spans the heights. */
pub const TILE: i64 = 128;
/** Sites whose light a level keeps, in PARTS parts, each locked and forgotten on its own. */
const KEPT_SITES: usize = 1 << 17;
const PARTS: usize = 64;
/** Tiles of a level a Light keeps, and that all keep in Shared, before forgetting them. */
const KEPT_TILES: usize = 16;
const SHARED_TILES: usize = 64;

/** A tile, made once: those that want it while it is made wait. */
type Tile = Arc<OnceLock<VoxelGrid>>;
/** Tiles by level and key. */
type Tiles = Map<(usize, (i64, i64)), Tile>;
/** The light coming to sites from each direction, as found. */
type Found = Map<[i64; 3], [Rgb; N]>;

/**
 * What the Lights of one world under one brightness share (a thread's
 * each), and with its host: tiles of every level, generated once, by
 * whichever needs one first, and the light at sites, found once.
 */
#[derive(Clone)]
pub struct Shared {
    brightness: Brightness,
    tiles: Arc<Mutex<Tiles>>,
    /** By level, its parts (PARTS). */
    light: Arc<Vec<Vec<Mutex<Found>>>>,
}

impl Shared {
    /** Nothing made or found yet, for a world under `brightness`. */
    pub fn new(brightness: Brightness) -> Shared {
        let light = (0..GRID_LEVELS).map(|_| (0..PARTS).map(|_| Mutex::default()).collect()).collect();
        Shared { brightness, tiles: Arc::default(), light: Arc::new(light) }
    }

    /** Tile `key` of `level` of the world `generator` makes, `heights` tall (World::heights_at), as its Lights read it. */
    pub fn tile(&self, generator: &Generator, level: u32, heights: (i64, i64), key: (i64, i64)) -> VoxelGrid {
        let tile = self.get(level as usize, key, || generate(generator, &tile_box(heights, key), level));
        tile.get().expect("made").clone()
    }

    /** Tile `key` of level `i`, made by `make` if not yet. */
    fn get(&self, i: usize, key: (i64, i64), make: impl FnOnce() -> VoxelGrid) -> Tile {
        let tile = {
            let mut tiles = self.tiles.lock().expect("tiles");
            if !tiles.contains_key(&(i, key)) && tiles.keys().filter(|(l, _)| *l == i).count() >= SHARED_TILES {
                tiles.retain(|(l, _), _| *l != i);
            }
            tiles.entry((i, key)).or_default().clone()
        };
        tile.get_or_init(make);
        tile
    }

    /** The part of level `i`'s light that `site`'s is kept in. */
    fn part(&self, i: usize, site: [i64; 3]) -> &Mutex<Found> {
        let mut hasher = Fast::default();
        site.iter().for_each(|&c| hasher.write_u64(c as u64));
        // Bits that the parts' maps do not place keys by.
        &self.light[i][(hasher.finish() >> 32) as usize % PARTS]
    }

    /** The light found at `site` of level `i`, if it was. */
    fn found(&self, i: usize, site: [i64; 3]) -> Option<[Rgb; N]> {
        self.part(i, site).lock().expect("light").get(&site).copied()
    }

    /** Keeps the light found at `site` of level `i`, its part forgotten first if full. */
    fn keep(&self, i: usize, site: [i64; 3], light: [Rgb; N]) {
        let mut part = self.part(i, site).lock().expect("light");
        if part.len() >= KEPT_SITES / PARTS {
            part.clear();
        }
        part.insert(site, light);
    }
}

/** How bright the sky and the sun are, as multiples of a clear day's, and glowing kinds, of a street lamp's. */
#[derive(Clone, Copy, Debug)]
pub struct Brightness {
    pub sky: f32,
    pub sun: f32,
    pub glow: f32,
}

/** A clear day's sky: its radiance (1 would light a surface facing up to its colour), and the sun's, high in the south-west. */
const SKY: Rgb = [0.55, 0.62, 0.75];
const SUN: Rgb = [2.6, 2.4, 2.1];
const SUN_DIRECTION: [i64; 3] = [-1, 1, 0];
/** Radiance of a glowing kind at glow 1, times its colour. */
const GLOW: f32 = 250.0;

/** A hasher for sites and tiles' keys, as FxHash: a rotate, a xor and a multiply per word; SipHash took much of the light's time. */
#[derive(Default)]
struct Fast(u64);

impl Hasher for Fast {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type Map<K, V> = HashMap<K, V, BuildHasherDefault<Fast>>;

/** A level's voxels, generated in tiles as they are needed. */
struct Level {
    heights: (i64, i64),
    tiles: Map<(i64, i64), Tile>,
    /** The tile read last, and its key: most reads are in it. */
    last: Option<((i64, i64), Tile)>,
}

pub struct Light {
    world: World,
    shared: Shared,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    unit: [[f64; 3]; N],
    /** Per face, how much the light from each direction weighs in the light on it (`on_face`). */
    cosines: [[f32; N]; N],
    /** Per direction, the face of the voxel it meets that it enters by. */
    enters: [usize; N],
    /** Per material id: its colour as a share of the light on it. */
    albedo: Vec<Rgb>,
    /** The radiance of a glow of GLOW_ONE. */
    glow: f32,
    /** The sky's radiance per direction (the sun's within it). */
    sky: [Rgb; N],
    /** What the sky looks like, where a picture's rays miss. */
    background: Rgb,
}

impl Light {
    /** The light of `world`, found only where asked, its tiles and light in `shared`. */
    pub fn new(world: World, shared: Shared) -> Light {
        let Brightness { sky, sun, glow } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest).map(|level| Level { heights: world.heights_at(level), tiles: Map::default(), last: None }).collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let cosines = unit.map(|normal| {
            let weights = unit.map(|u| dot(u, normal).max(0.0));
            let total: f64 = weights.iter().sum();
            weights.map(|w| (w / total) as f32)
        });
        let enters = DIRECTIONS.map(|d| DIRECTIONS.iter().position(|f| *f == d.map(|c| -c)).expect("faces come in pairs"));
        let materials = materials_of(&world.generator);
        // Light adds up in linear terms; colours are sRGB.
        let colour = |m: &crate::generator::Material| {
            let rgb = m.rgb();
            [16, 8, 0].map(|shift| srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0))
        };
        let albedo = materials.iter().map(colour).collect();
        let sun_at = DIRECTIONS.iter().position(|d| *d == SUN_DIRECTION).expect("the sun is in one of them");
        let skies = std::array::from_fn(|d| {
            let up = unit[d][1];
            let open = if up > 1e-9 {
                1.0
            } else if up > -1e-9 {
                0.5
            } else {
                0.0
            };
            std::array::from_fn(|c| open * SKY[c] * sky + if d == sun_at { SUN[c] * sun } else { 0.0 })
        });
        let background = SKY.map(|c| c * sky);
        let glow = GLOW * glow / GLOW_ONE as f32;
        Light { world, shared, levels, unit, cosines, enters, albedo, glow, sky: skies, background }
    }

    /** Whether a voxel is shown at `site` of level `i`, its tile generated if not yet. */
    fn shown(&mut self, i: usize, [x, y, z]: [i64; 3]) -> bool {
        let heights = self.levels[i].heights;
        (heights.0..heights.1).contains(&y) && self.hold(i, x, z).get(x, y, z) != 0
    }

    /** How light sees face `face` of `site` of level `i`: how much of it stops (alpha), its colour, what it glows. */
    fn see(&mut self, i: usize, [x, y, z]: [i64; 3], face: usize) -> (f32, Rgb, Rgb) {
        let heights = self.levels[i].heights;
        if !(heights.0..heights.1).contains(&y) {
            return (0.0, [0.0; 3], [0.0; 3]);
        }
        let (alpha, id, glow) = self.hold(i, x, z).light(x, y, z, face);
        (alpha as f32 / ALPHA as f32, self.albedo[id as usize], glow.map(|g| g as f32 * self.glow))
    }

    /** How much of what comes through `site` of level `i` it lets through. */
    fn open(&mut self, i: usize, [x, y, z]: [i64; 3]) -> f32 {
        let heights = self.levels[i].heights;
        if !(heights.0..heights.1).contains(&y) {
            return 1.0;
        }
        1.0 - self.hold(i, x, z).alpha(x, y, z) as f32 / ALPHA as f32
    }

    /** The tile of level `i` that holds column (x, z), from those shared, generated if not yet. */
    fn hold(&mut self, i: usize, x: i64, z: i64) -> &VoxelGrid {
        let key = (x.div_euclid(TILE), z.div_euclid(TILE));
        let level = &mut self.levels[i];
        if level.last.as_ref().is_none_or(|(k, _)| *k != key) {
            let tile = match level.tiles.get(&key) {
                Some(tile) => tile.clone(),
                None => {
                    let b = tile_box(level.heights, key);
                    let generator = &self.world.generator;
                    let tile = self.shared.get(i, key, || generate(generator, &b, i as u32));
                    level.tiles.insert(key, tile.clone());
                    tile
                }
            };
            level.last = Some((key, tile));
        }
        level.last.as_ref().expect("held").1.get().expect("made")
    }

    /**
     * The light coming to site `p` of level `i` from every direction:
     * past the neighbour, the coarser level's light (`near`), or at the
     * coarsest the sky (`march`).
     */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if let Some(light) = self.shared.found(i, p) {
            return light;
        }
        let light = if i + 1 == self.levels.len() { std::array::from_fn(|d| self.march(i, p, d)) } else { self.passed_on(i, p) };
        self.shared.keep(i, p, light);
        light
    }

    /** The light coming to `p` (a site of level `i`, not the coarsest) from each direction: from its neighbour that way, and past it. */
    fn passed_on(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        let (on, past) = self.near(i + 1, p);
        let heights = self.levels[i].heights;
        std::array::from_fn(|d| {
            let (v, q) = (DIRECTIONS[d], add(p, DIRECTIONS[d]));
            let (alpha, colour, glow) = self.see(i, q, self.enters[d]);
            // Above every voxel and not going down, or below them and not going up, nothing more is met.
            let beyond = if q[1] > heights.1 && v[1] >= 0 {
                self.sky[d]
            } else if q[1] < heights.0 - 1 && v[1] <= 0 {
                [0.0; 3]
            } else {
                past[d]
            };
            // Of what it stops, its colour times the light on its face towards p.
            let on_it = if alpha > 0.0 { self.on_face(&on, self.enters[d]) } else { [0.0; 3] };
            std::array::from_fn(|c| glow[c] + alpha * colour[c] * on_it[c] + (1.0 - alpha) * beyond[c])
        })
    }

    /** The light coming to `p` (a site of the coarsest level) from direction `d`, marching TOP_STEPS to the sky. */
    fn march(&mut self, i: usize, p: [i64; 3], d: usize) -> Rgb {
        let (v, heights) = (DIRECTIONS[d], self.levels[i].heights);
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], 1.0f32);
        let mut q = p;
        for _ in 0..TOP_STEPS {
            q = add(q, v);
            let (alpha, _, glow) = self.see(i, q, self.enters[d]);
            (0..3).for_each(|c| sum[c] += through * glow[c]);
            through *= 1.0 - alpha;
            if through < 1.0 / ALPHA as f32 || q[1] < heights.0 - 1 && v[1] <= 0 {
                return sum;
            }
            if q[1] > heights.1 && v[1] >= 0 {
                break;
            }
        }
        let sky = if v[1] >= 0 { self.sky[d] } else { [0.0; 3] };
        std::array::from_fn(|c| sum[c] + through * sky[c])
    }

    /**
     * The light at the sites of level `i` around `p`, a site of level
     * `i − 1`, each weighed by how much light it lets in (`around`):
     * all of it at p's centre, and each direction's where p's neighbour
     * that way ends.
     */
    fn near(&mut self, i: usize, p: [i64; 3]) -> ([Rgb; N], [Rgb; N]) {
        let (class, base) = class(p);
        let (mut on, mut on_total) = ([[0.0f32; 3]; N], 0.0f32);
        let (mut past, mut past_total) = ([[0.0f32; 3]; N], [0.0f32; N]);
        for (offset, weights) in &around()[class] {
            let site = add(base, *offset);
            let open = self.open(i, site);
            if open <= 0.0 {
                continue;
            }
            let light = self.light(i, site);
            let w = weights[N] * open;
            on_total += w;
            for (sum, l) in on.iter_mut().zip(&light) {
                (0..3).for_each(|c| sum[c] += w * l[c]);
            }
            for d in 0..N {
                let w = weights[d] * open;
                past_total[d] += w;
                (0..3).for_each(|c| past[d][c] += w * light[d][c]);
            }
        }
        let mean = |sum: Rgb, total: f32| if total > 0.0 { sum.map(|c| c / total) } else { sum };
        (on.map(|sum| mean(sum, on_total)), std::array::from_fn(|d| mean(past[d], past_total[d])))
    }

    /** Over π, the light on face `f` from `light`: its mean over the directions it faces, weighed by their cosines. */
    fn on_face(&self, light: &[Rgb; N], f: usize) -> Rgb {
        let mut sum = [0.0f32; 3];
        for (w, light) in self.cosines[f].iter().zip(light) {
            (0..3).for_each(|c| sum[c] += w * light[c]);
        }
        sum
    }

    /** Over π, the light on a surface facing `normal` from `light`: its mean over the directions it faces, weighed by their cosines. */
    fn on(&self, light: &[Rgb; N], normal: [f64; 3]) -> Rgb {
        let (mut sum, mut total) = ([0.0f64; 3], 0.0);
        for (unit, light) in self.unit.iter().zip(light) {
            let weight = dot(*unit, normal).max(0.0);
            (0..3).for_each(|c| sum[c] += weight * light[c] as f64);
            total += weight;
        }
        sum.map(|c| (c / total) as f32)
    }

    /**
     * The light from each face of the voxel at `site` of `level`: its
     * colour times the light on it, and what it glows. A voxel with no
     * empty neighbour (shown where a slice cuts it) takes the mean of its
     * neighbours' that have one, or none.
     */
    pub fn faces(&mut self, level: u32, site: [i64; 3]) -> FaceLight {
        let i = level as usize;
        let out = if self.surface(i, site) {
            self.lit(i, site)
        } else {
            let mut lit = Vec::new();
            for n in FACES.map(|f| add(site, f)) {
                if self.shown(i, n) && self.surface(i, n) {
                    lit.push(self.lit(i, n));
                }
            }
            std::array::from_fn(|f| std::array::from_fn(|c| lit.iter().map(|l| l[f][c]).sum::<f32>() / lit.len().max(1) as f32))
        };
        // Forgets what grew too much, to find it again if asked.
        for level in &mut self.levels {
            if level.tiles.len() > KEPT_TILES {
                level.tiles.clear();
            }
        }
        out
    }

    /** Whether the voxel at `site` of level `i` has an empty neighbour. */
    fn surface(&mut self, i: usize, site: [i64; 3]) -> bool {
        FACES.iter().any(|f| !self.shown(i, add(site, *f)))
    }

    /** The light from each face of the voxel at `site` of level `i`, on its surface. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> FaceLight {
        let light = self.light(i, site);
        std::array::from_fn(|f| {
            let on = self.on_face(&light, f);
            let (_, colour, glow) = self.see(i, site, f);
            std::array::from_fn(|c| colour[c] * on[c] + glow[c])
        })
    }

    /** What lights the voxel at `site` of `level` to its colour at 1: the light on its surface, which faces its empty neighbours. */
    pub fn on_voxel(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        let i = level as usize;
        let mut normal = [0.0; 3];
        for face in FACES {
            if !self.shown(i, add(site, face)) {
                (0..3).for_each(|c| normal[c] += face[c] as f64);
            }
        }
        let light = self.light(i, site);
        if normal == [0.0; 3] {
            // No surface, or all of it: the mean of all its faces'.
            let all = FACES.map(|f| self.on(&light, normalize(f.map(|v| v as f64))));
            return std::array::from_fn(|c| all.iter().map(|l| l[c]).sum::<f32>() / all.len() as f32);
        }
        self.on(&light, normalize(normal))
    }

    /** `picture`, of `level`, lit: each voxel's colour times the light on it, glowing ones' own light added; the sky where it misses. */
    pub fn relight(&mut self, picture: &mut Picture, level: u32) {
        for pixel in 0..picture.material.len() {
            let id = picture.material[pixel] as usize;
            let rgb = if id == 0 {
                self.background
            } else {
                let site = picture.sites[pixel];
                let on = self.on_voxel(level, site);
                let glow = self.see(level as usize, site, 0).2;
                std::array::from_fn(|c| self.albedo[id][c] * on[c] + glow[c])
            };
            for (c, value) in rgb.into_iter().enumerate() {
                picture.rgb[3 * pixel + c] = (255.0 * linear_to_srgb(value.clamp(0.0, 1.0))).round() as u8;
            }
        }
    }
}

/**
 * The coarser level's sites `near` reads, as offsets from the base of a
 * site's class, and their weights at the centre (the last) and a quarter
 * step along each direction: at each place, the nearest site and its
 * neighbours, 1.5 less the distance. Any further would leak light through
 * walls one site thick.
 */
type Around = Vec<([i64; 3], [f32; N + 1])>;

/** A site's class, of the 8 that the sites around its centre come in (`around`), and the base they are from: half of it, rounded down. */
fn class(p: [i64; 3]) -> (usize, [i64; 3]) {
    let base = p.map(|c| c.div_euclid(2));
    // Its last coordinate's parity follows from the others'.
    let class = (p[0] & 1) | (p[1] & 1) << 1 | ((base[0] + base[1] + base[2]) & 1) << 2;
    (class as usize, base)
}

fn around() -> &'static [Around; 8] {
    static AROUND: OnceLock<[Around; 8]> = OnceLock::new();
    AROUND.get_or_init(|| {
        std::array::from_fn(|class| {
            let (c, b) = (class as i64 & 3, class as i64 >> 2);
            // A site of the class: its base (b, 0, 0), its parities those of the class.
            let p = [2 * b + (c & 1), c >> 1, ((c & 1) + (c >> 1)) & 1];
            let base = [b, 0, 0];
            assert_eq!(self::class(p), (class, base));
            let centre = half(p);
            let at: [[f64; 3]; N + 1] =
                std::array::from_fn(|d| if d == N { centre } else { std::array::from_fn(|k| centre[k] + DIRECTIONS[d][k] as f64 / 4.0) });
            let mut sites = BTreeMap::new();
            for (k, x) in at.iter().enumerate() {
                let nearest = nearest_site(*x);
                for site in std::iter::once(nearest).chain(FACES.map(|f| add(nearest, f))) {
                    let r = sub(*x, site);
                    let weight = 1.5 - dot(r, r).sqrt();
                    if weight > 0.0 {
                        let offset = [0, 1, 2].map(|c| site[c] - base[c]);
                        sites.entry(offset).or_insert([0.0; N + 1])[k] = weight as f32;
                    }
                }
            }
            sites.into_iter().collect()
        })
    })
}

/** Tile `key`'s box, `heights` tall. */
fn tile_box(heights: (i64, i64), key: (i64, i64)) -> Box3 {
    Box3 { min: [key.0 * TILE, heights.0, key.1 * TILE], max: [(key.0 + 1) * TILE, heights.1, (key.1 + 1) * TILE] }
}

/** Where a site of one level is in the next coarser's units. */
fn half(p: [i64; 3]) -> [f64; 3] {
    p.map(|c| c as f64 / 2.0)
}

fn add(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [i64; 3]) -> [f64; 3] {
    [a[0] - b[0] as f64, a[1] - b[1] as f64, a[2] - b[2] as f64]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|c| c / length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{Field, Generator, KindDef, Layer, Node, Op, Population, SOLID, cuboid};

    const STONE: u8 = 0;
    const LAMP_KIND: u8 = 1;
    /** Lamps are cubes 4 units wide, as the towns' are: many sites, its glow on those inside. */
    const LAMP: f64 = 4.0;
    /** A lamp's mean shadow: its area over four. */
    const LAMP_SHADOW: f64 = 6.0 * LAMP * LAMP / 4.0;

    /**
     * Stone where `stone(x, y, z, spacing)` says, and white lamps at
     * `lamps`, as the towns' are; from y = −64 up to what the coarsest
     * levels' sites hold.
     */
    fn world(lamps: &[[f64; 3]], stone: impl Fn(i64, i64, i64, i64) -> bool + 'static) -> World {
        let kinds = vec![
            KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false },
            KindDef { name: "lamp", kind_type: 6, colors: &["#ffffff"], walk_through: false, glows: true },
        ];
        let shapes: Vec<_> =
            lamps.iter().map(|p| cuboid(p.map(|c| c - LAMP / 2.0), p.map(|c| c + LAMP / 2.0), LAMP_KIND, SOLID, Op::Fill)).collect();
        let lamps = Population::new("lamps", 1.0, 1024)
            .reach(1024)
            .anchor(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))
            .make(move |_, _| Some(Node::of(shapes.clone())));
        let ground = Field::at("stone", 0.0, Op::Fill, move |x, y, z, s| stone(x, y, z, s).then_some((STONE, None)));
        let layers = vec![Layer::Field(ground), Layer::Population(lamps)];
        World { generator: Generator { kinds, seed: 1, layers }, spawn: [0; 3], heights: (-64, 1024), points: Vec::new() }
    }

    /** A site on top of ground below y = 0: at y = −1, x + z odd. */
    fn ground(x: i64, z: i64) -> [i64; 3] {
        assert!((x + z) % 2 != 0, "({x}, {z}) is not on top");
        [x, -1, z]
    }

    #[test]
    fn the_sky_lights_open_ground_and_a_roof_shades_it() {
        let world = world(&[], |x, y, z, s| y < 0 || x.abs() <= 64 && z.abs() <= 64 && (24..24 + 2 * s).contains(&y));
        let mut light = Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 0.0, glow: 0.0 }));
        let open = light.on_voxel(0, ground(401, 400));
        let under = light.on_voxel(0, ground(1, 0));
        for c in 0..3 {
            assert!((open[c] / SKY[c] - 1.0).abs() < 0.1, "open ground {open:?}, the sky {SKY:?}");
        }
        // Under the middle of the roof, a sky between its edge and the horizon: an eighth of open ground's, the ground's light besides.
        assert!(under[1] < 0.3 * open[1], "under the roof {under:?}, open {open:?}");
    }

    #[test]
    fn walls_keep_the_sky_out() {
        // A closed box whose walls are one site thick at every level, as generators draw sheets.
        let world = world(&[], |x, y, z, s| {
            let wall = |c: i64| (48..48 + s).contains(&c.abs());
            let within = |c: i64| c.abs() < 48 + s;
            y < 0 || within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
        });
        let mut light = Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
        let inside = light.on_voxel(0, ground(1, 0));
        assert!(inside.iter().all(|c| *c < 0.02), "inside {inside:?}");
    }

    #[test]
    fn a_lamp_glows_as_much_at_every_level_it_is_smaller_than_a_site() {
        // Its glow is its share of its site, which is its area over the site's: times the site's area, the same.
        let world = world(&[[5.0, 31.0, 6.0]], |_, _, _, _| false);
        let glows: Vec<f64> = (3..=8)
            .map(|level| {
                let grid = generate(&world.generator, &Box3 { min: [-8; 3], max: [8; 3] }, level);
                grid.glow.values().map(|g| g[0] as f64).sum::<f64>() * (1u64 << (2 * level)) as f64
            })
            .collect();
        assert!(glows.iter().all(|g| (g / glows[0] - 1.0).abs() < 0.1), "glow times the site's area, by level from 3: {glows:?}");
    }

    #[test]
    fn a_closed_room_keeps_its_lamps_light_in() {
        // A box whose walls are one site thick at every level, a lamp inside, and the ground outside.
        let room = |x: i64, y: i64, z: i64, s: i64| {
            let wall = |c: i64| (16..16 + s).contains(&c.abs());
            let within = |c: i64| c.abs() < 16 + s;
            y < 0 || within(x) && within(z) && (0..32 + s).contains(&y) && (wall(x) || wall(z) || y >= 32)
        };
        let outside = |closed: bool| {
            let world = world(&[[0.0, 24.0, 0.0]], move |x, y, z, s| if closed { room(x, y, z, s) } else { y < 0 });
            let mut light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 }));
            light.on_voxel(0, ground(25, 0))[0]
        };
        let (open, closed) = (outside(false), outside(true));
        assert!(open > 0.0 && closed < 0.01 * open, "outside, open {open}, closed {closed}");
    }

    #[test]
    fn a_wall_is_lit_on_the_lamps_side() {
        // A wall one site thick across x, the lamp on its +x side.
        let world = world(&[[80.0, 10.0, 64.0]], |x, y, z, s| y < 0 || (64..64 + s).contains(&x) && y < 32 && (32..96).contains(&z));
        let light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 })).faces(0, [64, 10, 64]);
        let side = |sign: i64| FACES.iter().zip(&light).filter(|(f, _)| f[0] == sign).map(|(_, l)| l[0]).sum::<f32>();
        assert!(side(1) > 100.0 * side(-1), "towards the lamp {}, away {}", side(1), side(-1));
    }

    #[test]
    fn a_lamp_lights_the_ground_as_the_square_of_its_distance() {
        let height = 24.0;
        let world = world(&[[0.0, height, 0.0]], |_, y, _, _| y < 0);
        let mut light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 }));
        let mut ratios = Vec::new();
        for (x, z) in [(9, 0), (16, 7), (33, 0)] {
            let d = [x as f64, height + 1.0, z as f64];
            let d2 = dot(d, d);
            let expected = GLOW as f64 * LAMP_SHADOW * (d[1] / d2.sqrt()) / (std::f64::consts::PI * d2);
            ratios.push(light.on_voxel(0, ground(x, z))[0] as f64 / expected);
        }
        // Light from low over the horizon is seen along the nearest of the 12 directions, which may be flatter; further
        // off, the lamp is in a site as low as the ground's, its light comes flat, and none falls on it (docs/todo.md).
        assert!(ratios.iter().all(|r| (0.2..1.6).contains(r)), "measured over expected by distance {ratios:?}");
    }
}
