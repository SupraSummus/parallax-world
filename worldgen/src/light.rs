//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (fcc's light.rs).
//!
//! Light comes to a site from its 12 neighbours' directions: along each,
//! from the sites of its level, till the next coarser level can take over:
//! from STEPS on, where the coarser level has room for light (`coarser`).
//! Each site sends what it glows, and of what it stops (its alpha) its
//! colour times the light on it, also from the coarser level; the rest of
//! what comes from past it gets through. So each level's light comes from
//! coarser ones only, the coarsest's from the sky.
//!
//! Light sees the voxels as the eye does, in the grids hosts draw (Grid):
//! a coarse site its pieces fill in part shows a whole voxel, or nothing,
//! as often (VoxelGrid::show). What glows glows shown or not: a coarse
//! site a small lamp is in glows as bright as the lamp, on average over
//! its faces.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use fcc::GRID_LEVELS;
use fcc::light::{FaceLight, linear_to_srgb, srgb_to_linear};

use crate::World;
use crate::generator::{Generator, generate_shown, materials_of};
use crate::grid::{ALPHA, BRICK, Box3, GLOW_ONE, Grid};
use crate::picture::{FACES, Picture, nearest_site};

type Rgb = [f32; 3];

const N: usize = 12;

/** The directions light comes along: towards the neighbours, through the voxel's faces. */
const DIRECTIONS: [[i64; 3]; N] = FACES;

/** Steps along a direction before the coarser level's light may take over, and at most: at the coarsest, before the sky's does. */
const STEPS: usize = 1;
const TOP_STEPS: usize = 64;
/** Sites a tile is a side, at each level, as native's (tile_box). */
pub const TILE: i64 = 128;
/** Sites whose light a Light keeps at each level, all forgotten once full: the finer sites around read it soon after it is found, seldom later. */
const KEPT_SITES: usize = 1 << 13;

/** A tile, made once: those that want it while it is made wait. */
type Tile = Arc<OnceLock<Arc<Grid>>>;
/** Tiles by level and key. */
type Tiles = Map<(usize, [i64; 3]), Tile>;

/**
 * What the Lights of one world under one brightness share (a thread's
 * each), and with its host: tiles of every level, generated once, by
 * whichever needs one first, kept till the host forgets them (`forget`);
 * the host draws them too.
 */
#[derive(Clone)]
pub struct Shared {
    brightness: Brightness,
    tiles: Arc<Mutex<Tiles>>,
}

impl Shared {
    /** Nothing made yet, for a world under `brightness`. */
    pub fn new(brightness: Brightness) -> Shared {
        Shared { brightness, tiles: Arc::default() }
    }

    pub fn brightness(&self) -> Brightness {
        self.brightness
    }

    /** The same world under `brightness`: its tiles kept. */
    pub fn under(&self, brightness: Brightness) -> Shared {
        Shared { brightness, tiles: self.tiles.clone() }
    }

    /** Tile `key` of `level` of the world `generator` makes, of `heights` (World::heights_at). */
    pub fn tile(&self, generator: &Generator, level: u32, heights: (i64, i64), key: [i64; 3]) -> Arc<Grid> {
        let tile = self.tiles.lock().expect("tiles").entry((level as usize, key)).or_default().clone();
        tile.get_or_init(|| Arc::new(make_tile(generator, level, heights, key))).clone()
    }

    /** Forgets the tiles `far` says are far, by level and key, to make again if asked. */
    pub fn forget(&self, far: impl Fn(usize, [i64; 3]) -> bool) {
        self.tiles.lock().expect("tiles").retain(|&(level, key), _| !far(level, key));
    }
}

/** How bright the sky and the sun are, as multiples of a clear day's, and glowing kinds, of a street lamp's. */
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Brightness {
    pub sky: f32,
    pub sun: f32,
    pub glow: f32,
}

impl Brightness {
    /** The radiance of what glows, as a multiple of its colour (linear). */
    pub fn glowing(&self) -> f32 {
        GLOW * self.glow
    }
}

/** A clear day's sky: its radiance (1 would light a surface facing up to its colour), and the sun's, high in the south-west. */
const SKY: Rgb = [0.55, 0.62, 0.75];
const SUN: Rgb = [2.6, 2.4, 2.1];
const SUN_DIRECTION: [i64; 3] = [-1, 1, 0];
/** Radiance of a glowing kind at glow 1, times its colour. */
const GLOW: f32 = 250.0;

type Map<K, V> = fcc::FastMap<K, V>;

/** A level's voxels, generated in tiles as they are needed, and the light found at its sites. */
struct Level {
    heights: (i64, i64),
    /** The tile read last, and its key: most reads are in it. */
    last: Option<([i64; 3], Arc<Grid>)>,
    /** The light coming to sites from each direction, as found (KEPT_SITES). */
    found: Map<[i64; 3], [Rgb; N]>,
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

/** The coarser level's light around a site, as `along` takes it, read at once (`near`): per direction, on the face of what a step meets that faces the site, and past STEPS steps, if the coarser level has room for it there (`past`). */
struct Near {
    on: [Rgb; N],
    past: [Option<Rgb>; N],
}

impl Light {
    /** The light of `world`, found only where asked, its tiles and light in `shared`. */
    pub fn new(world: World, shared: Shared) -> Light {
        let Brightness { sky, sun, .. } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest).map(|level| Level { heights: world.heights_at(level), last: None, found: Map::default() }).collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let cosines = unit.map(|normal| {
            let weights = unit.map(|u| dot(u, normal).max(0.0));
            let total: f64 = weights.iter().sum();
            weights.map(|w| (w / total) as f32)
        });
        let enters = DIRECTIONS.map(|d| DIRECTIONS.iter().position(|f| *f == d.map(|c| -c)).expect("faces come in pairs"));
        let albedo = albedo(&world.generator);
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
        let glow = shared.brightness.glowing() / GLOW_ONE as f32;
        Light { world, shared, levels, unit, cosines, enters, albedo, glow, sky: skies, background }
    }

    /** Whether a whole voxel, one that lets nothing through, is at `site` of level `i`. */
    fn whole(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.open(i, site) <= 0.0
    }

    /** How light sees face `face` of `site` of level `i`: how much of it stops (alpha) and its colour; what it glows. */
    fn see(&mut self, i: usize, site: [i64; 3], face: usize) -> ((f32, Rgb), Rgb) {
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return ((0.0, [0.0; 3]), [0.0; 3]) };
        let ((id, alpha), glow) = (grid.face(x, y, z, face), grid.glow(x, y, z));
        ((alpha as f32 / ALPHA as f32, self.albedo[id as usize]), glow.map(|g| g as f32 * self.glow))
    }

    /** How much of what comes through `site` of level `i` it lets through. */
    fn open(&mut self, i: usize, site: [i64; 3]) -> f32 {
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return 1.0 };
        1.0 - grid.get(x, y, z).1 as f32 / ALPHA as f32
    }

    /** The tile of level `i` that holds `site`, from those shared, generated if not yet, and `site` as it takes it; none above or below every voxel. */
    fn hold(&mut self, i: usize, site: [i64; 3]) -> Option<([i32; 3], &Grid)> {
        let level = &mut self.levels[i];
        if !(level.heights.0..level.heights.1).contains(&site[1]) {
            return None;
        }
        let key = site.map(|v| v.div_euclid(TILE));
        if level.last.as_ref().is_none_or(|(k, _)| *k != key) {
            level.last = Some((key, self.shared.tile(&self.world.generator, i as u32, level.heights, key)));
        }
        Some((site.map(|v| v as i32), &level.last.as_ref().expect("held").1))
    }

    /** The light coming to site `p` of level `i` from every direction (`along`). */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if let Some(light) = self.levels[i].found.get(&p) {
            return *light;
        }
        let near = (i + 1 < self.levels.len()).then(|| self.near(i + 1, p));
        let light = std::array::from_fn(|d| self.along(i, p, d, near.as_ref()));
        let found = &mut self.levels[i].found;
        if found.len() == KEPT_SITES {
            found.clear();
        }
        found.insert(p, light);
        light
    }

    /**
     * The light coming to `p`, a site of level `i`, from direction `d`:
     * from the sites that way, each sending what it glows and, of what it
     * stops, its colour times the light on it (the coarser level's at
     * `p`); past them, the coarser level's light, from STEPS on, where it
     * has room for it (`past`). At the coarsest level (no `near`), from
     * the next TOP_STEPS, and past them the sky.
     */
    fn along(&mut self, i: usize, p: [i64; 3], d: usize, near: Option<&Near>) -> Rgb {
        let (v, heights) = (DIRECTIONS[d], self.levels[i].heights);
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], 1.0f32);
        let mut q = p;
        for step in 1..=TOP_STEPS {
            let from = q;
            q = add(q, v);
            let ((alpha, colour), glow) = self.see(i, q, self.enters[d]);
            let on = match near {
                Some(near) if alpha > 0.0 => near.on[d],
                _ => [0.0; 3],
            };
            (0..3).for_each(|c| sum[c] += through * (glow[c] + alpha * colour[c] * on[c]));
            through *= 1.0 - alpha;
            // Nothing more gets through, or nothing more is met: below every voxel and not going up, or above them and not going down.
            if through < 1.0 / ALPHA as f32 || q[1] < heights.0 - 1 && v[1] <= 0 {
                return sum;
            }
            if q[1] > heights.1 && v[1] >= 0 {
                return std::array::from_fn(|c| sum[c] + through * self.sky[d][c]);
            }
            if let Some(near) = near
                && step >= STEPS
                && let Some(past) = if step == STEPS { near.past[d] } else { self.past(i + 1, middle(from, q), d) }
            {
                return std::array::from_fn(|c| sum[c] + through * past[c]);
            }
        }
        let beyond = if near.is_none() && v[1] >= 0 { self.sky[d] } else { [0.0; 3] };
        std::array::from_fn(|c| sum[c] + through * beyond[c])
    }

    /**
     * Level `i`'s light from direction `d` at `at` (`reads`), if it has
     * room for light there, its site nearest empty. Where it fills what
     * the finer level leaves empty (a room whose walls, grown thick, close
     * it), the open sites near are not what lies past there.
     */
    fn past(&mut self, i: usize, at: [f64; 3], d: usize) -> Option<Rgb> {
        let mut reads = reads(at).peekable();
        let &(nearest, _) = reads.peek().expect("the nearest site");
        if self.open(i, nearest) <= 0.0 {
            return None;
        }
        let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
        for (site, w) in reads {
            let open = self.open(i, site);
            if open > 0.0 {
                let light = self.light(i, site)[d];
                total += w * open;
                (0..3).for_each(|c| sum[c] += w * open * light[c]);
            }
        }
        Some(sum.map(|c| c / total))
    }

    /** `along`'s readings of level `i` around `p`, a site of level `i − 1`, as `past` and the centre's light would give them, read at once from the sites around (`around`). */
    fn near(&mut self, i: usize, p: [i64; 3]) -> Near {
        let (class, base) = class(p);
        let (sites, nearest) = &around()[class];
        let (mut past, mut centre) = ([[0.0f32; 3]; N], [[0.0f32; 3]; N]);
        let mut totals = [0.0f32; N + 1];
        for (offset, weights) in sites {
            let site = add(base, *offset);
            let open = self.open(i, site);
            if open <= 0.0 {
                continue;
            }
            let light = self.light(i, site);
            for d in 0..N {
                let w = weights[d] * open;
                totals[d] += w;
                (0..3).for_each(|c| past[d][c] += w * light[d][c]);
            }
            let w = weights[N] * open;
            totals[N] += w;
            for (sum, l) in centre.iter_mut().zip(&light) {
                (0..3).for_each(|c| sum[c] += w * l[c]);
            }
        }
        let mean = |sum: Rgb, total: f32| if total > 0.0 { sum.map(|c| c / total) } else { sum };
        let centre = centre.map(|sum| mean(sum, totals[N]));
        Near {
            on: std::array::from_fn(|d| self.on_face(&centre, self.enters[d])),
            past: std::array::from_fn(|d| (self.open(i, add(base, nearest[d])) > 0.0).then(|| mean(past[d], totals[d]))),
        }
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
     * neighbour light gets through (shown where a slice cuts it) takes the
     * mean of its neighbours' that have one, or none.
     */
    pub fn faces(&mut self, level: u32, site: [i64; 3]) -> FaceLight {
        let i = level as usize;
        if self.surface(i, site) {
            return self.lit(i, site);
        }
        let mut lit = Vec::new();
        for n in FACES.map(|f| add(site, f)) {
            if self.whole(i, n) && self.surface(i, n) {
                lit.push(self.lit(i, n));
            }
        }
        std::array::from_fn(|f| std::array::from_fn(|c| lit.iter().map(|l| l[f][c]).sum::<f32>() / lit.len().max(1) as f32))
    }

    /** What lights the voxel at `site` of `level` (on_voxel), if light gets out of it: none if a ray meets it only where levels meet. */
    pub fn on_seen(&mut self, level: u32, site: [i64; 3]) -> Option<Rgb> {
        let i = level as usize;
        (self.open(i, site) > 0.0 || self.surface(i, site)).then(|| self.on_voxel(level, site))
    }

    /** Whether the voxel at `site` of level `i` has a neighbour light gets through. */
    fn surface(&mut self, i: usize, site: [i64; 3]) -> bool {
        FACES.iter().any(|f| !self.whole(i, add(site, *f)))
    }

    /** The light from each face of the voxel at `site` of level `i`, on its surface. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> FaceLight {
        let light = self.light(i, site);
        std::array::from_fn(|f| {
            let on = self.on_face(&light, f);
            let ((_, colour), glow) = self.see(i, site, f);
            std::array::from_fn(|c| colour[c] * on[c] + glow[c])
        })
    }

    /** What lights the voxel at `site` of `level` to its colour at 1: the light on its surface, which faces its empty neighbours. */
    pub fn on_voxel(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        let i = level as usize;
        let mut normal = [0.0; 3];
        for face in FACES {
            if !self.whole(i, add(site, face)) {
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
                let glow = self.see(level as usize, site, 0).1;
                std::array::from_fn(|c| self.albedo[id][c] * on[c] + glow[c])
            };
            for (c, value) in rgb.into_iter().enumerate() {
                picture.rgb[3 * pixel + c] = (255.0 * linear_to_srgb(value.clamp(0.0, 1.0))).round() as u8;
            }
        }
    }
}

/** Per material id: its colour as a share of the light on it, linear, as light adds up; colours are sRGB. */
fn albedo(generator: &Generator) -> Vec<Rgb> {
    let linear = |rgb: u32| [16, 8, 0].map(|shift| srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0));
    materials_of(generator).iter().map(|m| linear(m.rgb())).collect()
}

/**
 * Tile `key`'s box: a cube TILE sites a side, holding x in
 * [key[0] · TILE, (key[0] + 1) · TILE), y and z likewise, cut to `heights`
 * rounded out to whole bricks, so that every tile's bricks are on one
 * grid; empty (no y) if it is all above or below them.
 */
pub fn tile_box((low, high): (i64, i64), key: [i64; 3]) -> Box3 {
    let [x, y, z] = key.map(|k| k * TILE);
    let low = low.div_euclid(BRICK) * BRICK;
    let high = (high + BRICK - 1).div_euclid(BRICK) * BRICK;
    let (y0, y1) = (low.clamp(y, y + TILE), high.clamp(y, y + TILE));
    Box3 { min: [x, y0, z], max: [x + TILE, y1.max(y0), z + TILE] }
}

/** Tile `key` of `level` of the world `generator` makes, of `heights`, as hosts make and draw it (generate_shown), see-through ones too. */
pub fn make_tile(generator: &Generator, level: u32, heights: (i64, i64), key: [i64; 3]) -> Grid {
    generate_shown(generator, &tile_box(heights, key), level).into_grid(true)
}

fn add(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/**
 * The sites of a level that `at`, a point of it, reads light from, and
 * their weights: the site nearest first, and its neighbours, each 1.5 less
 * its distance. Any further would leak light through walls one site thick.
 */
fn reads(at: [f64; 3]) -> impl Iterator<Item = ([i64; 3], f32)> {
    let nearest = nearest_site(at);
    std::iter::once(nearest).chain(FACES.map(|f| add(nearest, f))).filter_map(move |site| {
        let r = sub(at, site);
        let w = 1.5 - dot(r, r).sqrt();
        (w > 0.0).then_some((site, w as f32))
    })
}

/**
 * `reads` at the places `near` reads around a site, by its class: as
 * offsets from the class's base, each site's weights at the N places past
 * STEPS steps along each direction and at the centre (the last); and the
 * site nearest each of the N.
 */
type Around = (Vec<([i64; 3], [f32; N + 1])>, [[i64; 3]; N]);

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
            let along = |d: usize, steps: usize| add(p, DIRECTIONS[d].map(|v| v * steps as i64));
            let at: [[f64; 3]; N + 1] =
                std::array::from_fn(|k| if k < N { middle(along(k, STEPS - 1), along(k, STEPS)) } else { middle(p, p) });
            let offset = |site: [i64; 3]| add(site, base.map(|c| -c));
            let mut sites = BTreeMap::new();
            for (k, at) in at.iter().enumerate() {
                for (site, w) in reads(*at) {
                    sites.entry(offset(site)).or_insert([0.0; N + 1])[k] = w;
                }
            }
            let nearest = std::array::from_fn(|d| offset(nearest_site(at[d])));
            (sites.into_iter().collect(), nearest)
        })
    })
}

/** Where the middle of sites `a` and `b` of a level is at the next coarser one. */
fn middle(a: [i64; 3], b: [i64; 3]) -> [f64; 3] {
    std::array::from_fn(|c| (a[c] + b[c]) as f64 / 4.0)
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
mod reference;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{Field, Generator, KindDef, Layer, Node, Op, Population, SOLID, cuboid};
    use fcc::light::colour;

    #[test]
    fn tiles_are_cubes_cut_to_the_heights_at_bricks() {
        let heights = (-70, 140);
        let cube = |key: [i64; 3]| Box3 { min: key.map(|k| k * TILE), max: key.map(|k| (k + 1) * TILE) };
        assert_eq!(tile_box(heights, [2, 1, -3]), Box3 { max: [3 * TILE, 144, -2 * TILE], ..cube([2, 1, -3]) });
        assert_eq!(tile_box(heights, [0, -1, 0]), Box3 { min: [0, -72, 0], ..cube([0, -1, 0]) });
        assert_eq!(tile_box((-500, 500), [1, 2, 3]), cube([1, 2, 3]), "whole within them");
        for key in [[0, 2, 0], [0, -2, 0]] {
            let b = tile_box(heights, key);
            assert!(b.min[1] == b.max[1], "{key:?}: none of it in the heights");
        }
    }

    const STONE: u8 = 0;
    const LAMP_KIND: u8 = 1;
    /** Lamps are cubes 4 units wide, as the towns' are: many sites, its glow on those inside. */
    const LAMP: f64 = 4.0;

    /**
     * Stone where `stone(x, y, z, spacing)` says (the sites' spacing), and white lamps at
     * `lamps`, as the towns' are; from y = −64 up to what the coarsest
     * levels' sites hold.
     */
    fn world(lamps: &[[f64; 3]], stone: impl Fn(i64, i64, i64, i64) -> bool + 'static) -> World {
        let kinds = vec![
            KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false, medium: false },
            KindDef { name: "lamp", kind_type: 6, colors: &["#ffffff"], walk_through: false, glows: true, medium: false },
        ];
        let shapes: Vec<_> =
            lamps.iter().map(|p| cuboid(p.map(|c| c - LAMP / 2.0), p.map(|c| c + LAMP / 2.0), LAMP_KIND, SOLID, Op::Fill)).collect();
        let lamps = Population::new("lamps", 1.0, 1024)
            .reach(1024)
            .anchor(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))
            .make(move |_, _| Some(Node::of(shapes.clone())));
        let ground = Field::at("stone", 0.0, Op::Fill, move |x, y, z| stone(x, y, z, crate::plan::spacing()).then_some((STONE, None)));
        let layers = vec![Layer::Field(ground), Layer::Population(lamps)];
        World { generator: Generator { kinds, seed: 1, layers }, spawn: [0; 3], heights: (-64, 1024), points: Vec::new() }
    }

    /** A site on top of ground below y = 0: at y = −1, x + z odd. */
    fn ground(x: i64, z: i64) -> [i64; 3] {
        assert!((x + z) % 2 != 0, "({x}, {z}) is not on top");
        [x, -1, z]
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
                let grid = crate::generator::generate(&world.generator, &Box3 { min: [-8; 3], max: [8; 3] }, level);
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

    /** A scene to hold Light to the reference: its world, how bright, the ground sites to probe, rays per face, and the bound on Light's error there. */
    struct Scene {
        name: String,
        world: World,
        brightness: Brightness,
        probes: Vec<[i64; 3]>,
        samples: u64,
        bound: f64,
    }

    fn scenes() -> Vec<Scene> {
        let day = Brightness { sky: 1.0, sun: 1.0, glow: 0.0 };
        let sky = Brightness { sky: 1.0, sun: 0.0, glow: 0.0 };
        let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
        let scene =
            |name: &str, world, brightness, probes, samples, bound| Scene { name: name.into(), world, brightness, probes, samples, bound };
        let mut scenes = vec![
            scene("open ground", world(&[], |_, y, _, _| y < 0), sky, vec![ground(1, 0)], 4096, 0.1),
            scene(
                "wall, sun",
                world(&[], |x, y, z, s| y < 0 || (0..2 * s).contains(&x) && y < 32 && z.abs() < 96),
                day,
                [-9, -1, 3, 17, 29, 41].map(|x| ground(x, 0)).to_vec(),
                4096,
                0.2,
            ),
            scene(
                "lamp",
                world(&[[0.0, 24.0, 0.0]], |_, y, _, _| y < 0),
                night,
                vec![ground(1, 0), ground(16, 7), ground(33, 0)],
                1 << 16,
                1.6,
            ),
        ];
        // A roof 2 sites thick and 128 units wide at y = 24, turned about y, probed across it: the same light at every turn.
        for (turn, bound) in [(0.0, 0.35), (22.5, 0.35), (45.0, 0.55)] {
            let (sin, cos) = f64::to_radians(turn).sin_cos();
            let roof = world(&[], move |x, y, z, s| {
                let (u, v) = (cos * x as f64 + sin * z as f64, cos * z as f64 - sin * x as f64);
                y < 0 || u.abs() <= 64.0 && v.abs() <= 64.0 && (24..24 + 2 * s).contains(&y)
            });
            let probes = [0.0, 40.0, 56.0, 72.0, 96.0].map(|d: f64| ground_near(d * cos, d * sin)).to_vec();
            scenes.push(scene(&format!("roof turned {turn}°"), roof, sky, probes, 4096, bound));
        }
        // A yard walled 48 high, open to the sky, and past a wall with a low door a room roofed at 48: from 32 past the wall, the sky does not show through the door, so only light that bounced comes there.
        let yard = world(&[], |x, y, z, s| {
            let inside = x.abs() < 64 && z.abs() < 48 && y < 48;
            let shell = x.abs() < 64 + 2 * s && z.abs() < 48 + 2 * s && y < 48 + if x >= 0 { 2 * s } else { 0 };
            let wall = (0..2 * s).contains(&x) && !(z.abs() < 12 && y < 16);
            y < 0 || shell && (!inside || wall)
        });
        let probes = [-48.0, -24.0, 32.0, 44.0, 56.0].map(|x| ground_near(x, 0.0)).to_vec();
        scenes.push(scene("yard, past a wall", yard, sky, probes, 1 << 16, 1.8));
        scenes
    }

    /** The ground site nearest (x, z). */
    fn ground_near(x: f64, z: f64) -> [i64; 3] {
        let (x, z) = (x.round() as i64, z.round() as i64);
        ground(x + (x + z + 1).rem_euclid(2), z)
    }

    /** What the open faces of the voxel at `site` (level 0) send out by Light, on average. */
    fn lit(light: &mut Light, site: [i64; 3]) -> Rgb {
        let open: Vec<usize> = (0..N).filter(|&f| !light.whole(0, add(site, FACES[f]))).collect();
        let faces = light.faces(0, site);
        std::array::from_fn(|c| open.iter().map(|&f| faces[f][c]).sum::<f32>() / open.len().max(1) as f32)
    }

    /** Where the reference's light at the scenes' probes is kept (`reference_values`), as lines of scene, site, light and its standard error. */
    const VALUES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/light-reference.txt");

    fn line(name: &str, [x, y, z]: [i64; 3]) -> String {
        format!("{name}; {x} {y} {z}")
    }

    /** The reference's light at the scenes' probes, written to VALUES: run by hand when the scenes change, as it takes a while. It finds only the probes VALUES lacks; delete a changed scene's lines first. */
    #[test]
    #[ignore]
    fn reference_values() {
        let kept = std::fs::read_to_string(VALUES).unwrap_or_default();
        let mut lines = String::new();
        for Scene { name, world, brightness, probes, samples, .. } in scenes() {
            let reference = reference::Reference::new(&world, brightness);
            for site in probes {
                let key = format!("{}; ", line(&name, site));
                lines += &match kept.lines().find(|l| l.starts_with(&key)) {
                    Some(l) => format!("{l}\n"),
                    None => {
                        let ([r, g, b], error) = reference.voxel(site, samples);
                        format!("{key}{r} {g} {b}; {error}\n")
                    }
                };
            }
        }
        std::fs::write(VALUES, lines).expect("written");
    }

    /**
     * Light against the reference (reference.rs, as `reference_values`
     * kept it), scene by scene: how far off the light from each probe's
     * open faces is, as the root mean square of ln(light / reference)
     * over the probes. The bounds are Light's errors as they were; lower
     * one as Light gets closer. `-- --nocapture` prints each probe.
     */
    #[test]
    fn against_the_reference() {
        let kept = std::fs::read_to_string(VALUES).expect("the reference's values: run reference_values");
        let value = |name: &str, site| {
            let key = line(name, site);
            let found = kept
                .lines()
                .find_map(|l| l.strip_prefix(&key)?.strip_prefix("; "))
                .unwrap_or_else(|| panic!("{key}: run reference_values"));
            let [light, error] = [0, 1].map(|k| found.split("; ").nth(k).expect("light; error"));
            (light.split(' ').map(|v| v.parse::<f32>().expect("a number")).sum::<f32>(), error.parse::<f32>().expect("a number"))
        };
        let mut over = Vec::new();
        for Scene { name, world, brightness, probes, bound, .. } in scenes() {
            let mut light = Light::new(world, Shared::new(brightness));
            let mut squares = 0.0;
            for &site in &probes {
                let (m, (r, error)) = (lit(&mut light, site).iter().sum::<f32>(), value(&name, site));
                println!("{name}, {site:?}: light {m:.4}, reference {r:.4} ± {error:.4}, light / reference {:.2}", m / r);
                squares += ((m / r) as f64).ln().powi(2);
            }
            let rms = (squares / probes.len() as f64).sqrt();
            println!("{name}: Light is off by {rms:.2} (ln), at most {bound}");
            if rms > bound {
                over.push(name);
            }
        }
        assert!(over.is_empty(), "Light is further off than it was in {over:?}");
    }

    /**
     * Maps of the ground around each scene, from above, as PNGs in the
     * directory OUT: Light's, the reference's (a sixty-fourth of the rays,
     * kept in OUT for the next run) and where Light is brighter (red) or
     * darker (blue), up to e^1.5 times; and how far off Light is over
     * each map, as `against_the_reference` reckons.
     * Run by hand: OUT=dir cargo test --release maps -- --ignored
     */
    #[test]
    #[ignore]
    fn maps() {
        const HALF: i64 = 96;
        let out = std::env::var("OUT").expect("OUT, a directory");
        let (w, scale, gap) = (HALF as usize, 4, 4);
        for Scene { name, world, brightness, samples, .. } in scenes() {
            let file = format!("{out}/{}", name.replace([' ', ',', '°'], ""));
            let sites: Vec<[i64; 3]> =
                (0..w * w).map(|k| ground_near(2.0 * (k % w) as f64 - HALF as f64, HALF as f64 - 2.0 * (k / w) as f64)).collect();
            let kept: Vec<f32> = std::fs::read(format!("{file}.reference"))
                .unwrap_or_default()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let references: Vec<Rgb> = if kept.len() == 3 * sites.len() {
                kept.as_chunks::<3>().0.to_vec()
            } else {
                let reference = reference::Reference::new(&world, brightness);
                let found: Vec<Rgb> = sites.iter().map(|&site| reference.voxel(site, samples / 64).0).collect();
                std::fs::write(format!("{file}.reference"), found.iter().flatten().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
                    .expect("kept");
                found
            };
            let mut light = Light::new(world, Shared::new(brightness));
            let lights: Vec<Rgb> = sites.iter().map(|&site| lit(&mut light, site)).collect();
            let logs: Vec<f64> = lights
                .iter()
                .zip(&references)
                .map(|(m, r)| (m.iter().sum::<f32>() / r.iter().sum::<f32>()).ln() as f64)
                .filter(|v| v.is_finite())
                .collect();
            println!(
                "{name}: Light is off by {:.2} (ln, over the map)",
                (logs.iter().map(|v| v * v).sum::<f64>() / logs.len() as f64).sqrt()
            );
            // Exposed so that the reference's brightest tenth is white.
            let mut sums: Vec<f32> = references.iter().map(|r| r.iter().sum()).collect();
            sums.sort_by(f32::total_cmp);
            let exposure = 3.0 / sums[sums.len() * 9 / 10].max(1e-9);
            let shown = |l: Rgb| colour(l.map(|c| c * exposure)).to_le_bytes();
            let off = |m: Rgb, r: Rgb| {
                let ratio = (m.iter().sum::<f32>() / r.iter().sum::<f32>()).ln();
                let ratio = if ratio.is_nan() { 0.0 } else { ratio.clamp(-1.5, 1.5) / 1.5 };
                let pale = (255.0 * (1.0 - ratio.abs())) as u8;
                if ratio > 0.0 { [255, pale, pale, 255] } else { [pale, pale, 255, 255] }
            };
            let width = 3 * w + 2 * gap;
            let mut rgb = vec![255; width * w * 3];
            for (k, (&m, &r)) in lights.iter().zip(&references).enumerate() {
                for (panel, pixel) in [shown(m), shown(r), off(m, r)].into_iter().enumerate() {
                    let at = 3 * ((k / w) * width + panel * (w + gap) + k % w);
                    rgb[at..at + 3].copy_from_slice(&pixel[..3]);
                }
            }
            let rgb = (0..w * scale)
                .flat_map(|y| (0..width * scale).flat_map(move |x| [0, 1, 2].map(|c| ((y / scale) * width + x / scale) * 3 + c)))
                .map(|at| rgb[at])
                .collect();
            let picture = Picture { width: width * scale, height: w * scale, rgb, ..Picture::default() };
            std::fs::write(format!("{file}.png"), picture.png()).expect("written");
        }
    }
}
