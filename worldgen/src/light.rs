//! Light on the lattices 2^k·FCC: what reaches each site from the sky, the
//! sun and glowing kinds, and from the surfaces they light. A prototype,
//! for pictures (look's `light`).
//!
//! Light comes to a site along 18 directions: towards its 12 neighbours
//! and along the 6 axes. Along each it comes from one step of the site's
//! own level, and beyond that from the light of the next coarser level
//! where the step ends. A surface met on the way sends back its colour
//! times the light on it, which also comes from the coarser level; glowing
//! sites passed or met give their glow. So each level's light is made from
//! coarser levels' only, and the coarsest's from the sky.
//!
//! What glows goes the other way, up: a site's glow is that of the finer
//! level's sites in its cell, summed. So many weak lamps far away add up,
//! and one is not lost where the coarser grid has no voxel for it:
//! generators keep a small thing at a coarse level only by chance, which
//! keeps its shadow on average but not its light. Glow is an intensity
//! (light per solid angle, as a lamp's radiance times its shadow): a line
//! of sites stands for the area across it that each line does, so glow met
//! along it is seen as spread over that area. Summed, that is as bright as
//! the lamps on average; one at a time it is brighter or darker by where
//! the lines pass, so a lamp is seen sharp only near.

use std::collections::{HashMap, HashSet};

use crate::World;
use crate::generator::{generate, materials_of};
use crate::grid::{BRICK, Box3, VoxelGrid};
use crate::picture::{FACES, Picture, nearest_site};

type Rgb = [f32; 3];

const N: usize = 18;

/** The directions light comes along, as lattice vectors: towards the 12 neighbours (as FACES), then the 6 axes. */
const DIRECTIONS: [[i64; 3]; N] = {
    let axes = [[2, 0, 0], [-2, 0, 0], [0, 2, 0], [0, -2, 0], [0, 0, 2], [0, 0, -2]];
    let mut directions = [[0; 3]; N];
    let mut k = 0;
    while k < N {
        directions[k] = if k < FACES.len() { FACES[k] } else { axes[k - FACES.len()] };
        k += 1;
    }
    directions
};

/** Steps along a direction at a level before the coarser level's light takes over; at the coarsest, before the sky does. */
const STEPS: usize = 1;
const TOP_STEPS: usize = 64;
/** Levels above the base: the coarsest's steps reach about 2^(LEVELS + 1) of the base's units. */
const LEVELS: u32 = 9;
/**
 * Levels from the base that hold glow: lamps light what is up to about
 * 2^(GLOWING + 1) of the base's units away. Their glow is summed from the
 * base level's tiles, which costs generating those (up to about
 * 2^(GLOWING + 2) around what is lit); the engine might instead give how
 * many glowing things a cell of any size holds, as Points do. Above level
 * 0 the base's grid has already lost lamps by chance.
 */
const GLOWING: usize = 6;

/** Sites per tile in x and z, at each level; a tile spans the heights. */
const TILE: i64 = 64;

/** How bright the sky and the sun are and how much glowing kinds give, as multiples of a clear day's. */
#[derive(Clone, Copy, Debug)]
pub struct Sky {
    pub sky: f32,
    pub sun: f32,
    pub glow: f32,
}

/** A clear day's sky: its radiance (1 would light a surface facing up to its colour), and the sun's, high in the south-west. */
const SKY: Rgb = [0.55, 0.62, 0.75];
const SUN: Rgb = [2.6, 2.4, 2.1];
const SUN_DIRECTION: [i64; 3] = [-1, 1, 0];
/** Radiance of a glowing kind at glow 1, times its colour. */
const GLOW: f32 = 500.0;
/** A voxel's mean shadow: a quarter of its area (Cauchy), for a rhombic dodecahedron of volume 2. */
const SHADOW: f32 = 2.121_320_3;

/** A level's voxels, generated in tiles as they are needed; the glow of its sites (none: none) and the light at them, as far as found. */
struct Level {
    heights: (i64, i64),
    tiles: HashMap<(i64, i64), VoxelGrid>,
    glow: HashMap<[i64; 3], Rgb>,
    light: HashMap<[i64; 3], [Rgb; N]>,
}

pub struct Light<'w> {
    world: &'w World,
    /** Level `base + i` is `levels[i]`; the last is the coarsest. */
    base: u32,
    levels: Vec<Level>,
    unit: [[f64; 3]; N],
    /** The solid angle each direction stands for: the share of the sphere nearer to it than to any other, of 4π. */
    solid_angle: [f64; N],
    /** Per direction, the face of the voxel it meets that it enters by: the one facing most against it. */
    enters: [usize; N],
    /** Per material id: its colour as a share of the light on it, and what it gives itself. */
    albedo: Vec<Rgb>,
    emission: Vec<Rgb>,
    /** The sky's radiance per direction (the sun's within it). */
    sky: [Rgb; N],
    /** Per direction, over the area across it that a line of the base level's sites along it stands for: half its length. */
    per_area: [f32; N],
    /** The base level's tiles whose glow has been summed into the glowing levels'. */
    scanned: HashSet<(i64, i64)>,
}

impl<'w> Light<'w> {
    /** The light of `world` at `base` and the levels above it, under `sky`; found only where asked. */
    pub fn new(world: &'w World, base: u32, sky: Sky) -> Light<'w> {
        let levels = (base..=base + LEVELS)
            .map(|level| Level { heights: world.heights_at(level), tiles: HashMap::new(), glow: HashMap::new(), light: HashMap::new() })
            .collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let enters = DIRECTIONS.map(|d| {
            let along = |f: &[i64; 3]| f[0] * d[0] + f[1] * d[1] + f[2] * d[2];
            (0..FACES.len()).min_by_key(|&f| along(&FACES[f])).expect("faces")
        });
        let materials = materials_of(&world.generator);
        // Light adds up in linear terms; colours are sRGB.
        let colour = |m: &crate::generator::Material| {
            let rgb = m.rgb();
            [16, 8, 0].map(|shift| linear(((rgb >> shift) & 255) as f32 / 255.0))
        };
        let albedo = materials.iter().map(colour).collect();
        let emission = materials.iter().map(|m| if m.glows { colour(m).map(|c| c * GLOW * sky.glow) } else { [0.0; 3] }).collect();
        let sun = DIRECTIONS.iter().position(|d| *d == SUN_DIRECTION).expect("the sun is in one of them");
        let skies = std::array::from_fn(|d| {
            let up = unit[d][1];
            let open = if up > 1e-9 {
                1.0
            } else if up > -1e-9 {
                0.5
            } else {
                0.0
            };
            std::array::from_fn(|c| open * SKY[c] * sky.sky + if d == sun { SUN[c] * sky.sun } else { 0.0 })
        });
        let solid_angle = solid_angles(&unit);
        // Lines of sites |v| apart along v, half a site per unit of volume: one per 2 / |v| of area across them.
        let per_area = DIRECTIONS.map(|v| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f32).sqrt() / 2.0);
        Light { world, base, levels, unit, solid_angle, enters, albedo, emission, sky: skies, per_area, scanned: HashSet::new() }
    }

    /** The material of face `face` of the voxel at `site` of level `base + i` (0: empty). */
    fn face(&mut self, i: usize, [x, y, z]: [i64; 3], face: usize) -> u8 {
        let (world, level) = (self.world, self.base + i as u32);
        let Level { heights, tiles, .. } = &mut self.levels[i];
        if y < heights.0 || y >= heights.1 {
            return 0;
        }
        let key = (x.div_euclid(TILE), z.div_euclid(TILE));
        let b = Box3 { min: [key.0 * TILE, heights.0, key.1 * TILE], max: [(key.0 + 1) * TILE, heights.1, (key.1 + 1) * TILE] };
        tiles.entry(key).or_insert_with(|| generate(&world.generator, &b, level)).face(x, y, z, face)
    }

    fn empty(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.face(i, site, 0) == 0
    }

    /**
     * The material a step from `from` along direction `d` meets, if any:
     * the voxel it ends in, or along an axis, the four it passes between
     * (they meet at a corner on its way) if all are there.
     */
    fn step(&mut self, i: usize, from: [i64; 3], d: usize) -> Option<([i64; 3], u8)> {
        let v = DIRECTIONS[d];
        let to = add(from, v);
        if d >= FACES.len() {
            let middle = [0, 1, 2].map(|c| from[c] + v[c] / 2);
            let axis = v.iter().position(|c| *c != 0).expect("an axis");
            let around = [(1, 1), (1, -1), (2, 1), (2, -1)].map(|(other, sign)| {
                let mut site = middle;
                site[(axis + other) % 3] += sign;
                site
            });
            if around.iter().all(|&site| !self.empty(i, site)) {
                return Some((around[0], self.face(i, around[0], self.enters[d])));
            }
        }
        match self.face(i, to, self.enters[d]) {
            0 => None,
            id => Some((to, id)),
        }
    }

    /** The light coming to `p` (a site of level `base + i`) from direction `d`. */
    fn incoming(&mut self, i: usize, p: [i64; 3], d: usize) -> Rgb {
        let coarsest = i + 1 == self.levels.len();
        let (v, heights) = (DIRECTIONS[d], self.levels[i].heights);
        let (mut q, mut glows) = (p, [0.0; 3]);
        for _ in 0..if coarsest { TOP_STEPS } else { STEPS } {
            let (front, met) = (q, self.step(i, q, d));
            q = add(q, v);
            let glow = self.levels[i].glow.get(&met.map_or(q, |(site, _)| site)).copied().unwrap_or_default();
            // Lines of a level's sites stand for 4^i times the base's area.
            let seen = self.per_area[d] / (1u32 << (2 * i)) as f32;
            (0..3).for_each(|c| glows[c] += glow[c] * seen);
            if let Some((_, id)) = met {
                // What the surface met sends back: its colour times the light on it, from the coarser level.
                let on_it = if coarsest { [0.0; 3] } else { self.irradiance(i + 1, half(front), self.unit[d].map(|c| -c)) };
                let albedo = self.albedo[id as usize];
                return std::array::from_fn(|c| glows[c] + albedo[c] * on_it[c]);
            }
            // Above every voxel and not going down, or below them and not going up, nothing more is met.
            if q[1] > heights.1 && v[1] >= 0 {
                return std::array::from_fn(|c| glows[c] + self.sky[d][c]);
            }
            if q[1] < heights.0 - 1 && v[1] <= 0 {
                return glows;
            }
        }
        // The coarser level's light from where its first step's voxel begins as this level's last one's ends.
        let beyond = if !coarsest {
            self.near(i + 1, [0, 1, 2].map(|c| (2 * q[c] - v[c]) as f64 / 4.0), None)[d]
        } else if v[1] >= 0 {
            self.sky[d]
        } else {
            [0.0; 3]
        };
        std::array::from_fn(|c| glows[c] + beyond[c])
    }

    /** The light coming to site `p` of level `base + i` from every direction. */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if let Some(light) = self.levels[i].light.get(&p) {
            return *light;
        }
        self.scan_around(i, p);
        let mut light = [[0.0; 3]; N];
        for (d, l) in light.iter_mut().enumerate() {
            *l = self.incoming(i, p, d);
        }
        self.levels[i].light.insert(p, light);
        light
    }

    /**
     * Sums the glow of the base level's tiles around site `p` of level
     * `base + i` into the glowing levels', if not yet: all that a step from `p`
     * may meet holds. A coarse site's glow comes from the base level's
     * sites within 2^(i + 1) of it, and a step reaches as far.
     */
    fn scan_around(&mut self, i: usize, p: [i64; 3]) {
        if i >= GLOWING {
            return;
        }
        let reach = (4 << i) + 4;
        let [x, z] = [p[0] << i, p[2] << i];
        for tx in (x - reach).div_euclid(TILE)..=(x + reach).div_euclid(TILE) {
            for tz in (z - reach).div_euclid(TILE)..=(z + reach).div_euclid(TILE) {
                if self.scanned.insert((tx, tz)) {
                    self.scan((tx, tz));
                }
            }
        }
    }

    /** Sums the glow of the base level's tile `key` into the glowing levels'. */
    fn scan(&mut self, key: (i64, i64)) {
        let heights = self.levels[0].heights;
        self.face(0, [key.0 * TILE, heights.0, key.1 * TILE], 0);
        let grid = &self.levels[0].tiles[&key];
        let [bx, by] = [0, 1].map(|c| (grid.n[c] + BRICK - 1) / BRICK);
        let mut glows: HashMap<[i64; 3], Rgb> = HashMap::new();
        for (b, at) in grid.bricks.iter().enumerate() {
            if *at < 0 {
                continue;
            }
            let b = b as i64;
            let corner = [b % bx, b / bx % by, b / (bx * by)].map(|c| c * BRICK);
            for x in 0..BRICK {
                for y in 0..BRICK {
                    for z in 0..BRICK {
                        let site = [grid.min[0] + corner[0] + x, grid.min[1] + corner[1] + y, grid.min[2] + corner[2] + z];
                        let radiance = self.emission[grid.get(site[0], site[1], site[2]) as usize];
                        if radiance != [0.0; 3] {
                            glows.insert(site, radiance.map(|c| c * SHADOW));
                        }
                    }
                }
            }
        }
        for i in 0..GLOWING {
            let mut coarser: HashMap<[i64; 3], Rgb> = HashMap::new();
            for (site, glow) in glows {
                let sum = self.levels[i].glow.entry(site).or_default();
                (0..3).for_each(|c| sum[c] += glow[c]);
                // Its share in each cell it is in: all of one at its centre, half of each of two on a face, a sixth of each of six at a corner.
                let s = nearest_site(half(site));
                for cell in std::iter::once(s).chain(DIRECTIONS.iter().map(|d| add(s, *d))) {
                    let r = [0, 1, 2].map(|c| site[c] - 2 * cell[c]);
                    let share = match r[0] * r[0] + r[1] * r[1] + r[2] * r[2] {
                        0 => 1.0,
                        2 => 0.5,
                        4 => 1.0 / 6.0,
                        _ => continue,
                    };
                    let sum = coarser.entry(cell).or_default();
                    (0..3).for_each(|c| sum[c] += glow[c] * share);
                }
            }
            glows = coarser;
        }
    }

    /**
     * The light coming to point `x` (level `base + i`'s units) from every
     * direction: that of the empty sites around it, the nearer the more;
     * if given a side (unit), only those on that side of it.
     */
    fn near(&mut self, i: usize, x: [f64; 3], side: Option<[f64; 3]>) -> [Rgb; N] {
        let s = nearest_site(x);
        let (mut sum, mut total) = ([[0.0f32; 3]; N], 0.0);
        for site in std::iter::once(s).chain(FACES.iter().map(|f| add(s, *f))) {
            let r = sub(x, site);
            let weight = (1.5 - dot(r, r).sqrt()) as f32;
            if weight <= 0.0 || side.is_some_and(|side| dot(r, side) > 0.25) || !self.empty(i, site) {
                continue;
            }
            let light = self.light(i, site);
            for (sum, light) in sum.iter_mut().zip(light) {
                (0..3).for_each(|c| sum[c] += weight * light[c]);
            }
            total += weight;
        }
        if total > 0.0 { sum.map(|l| l.map(|c| c / total)) } else { sum }
    }

    /** The light on a surface at `x` (level `base + i`'s units) facing `normal` (unit), over π: what lights it to its colour at 1. */
    fn irradiance(&mut self, i: usize, x: [f64; 3], normal: [f64; 3]) -> Rgb {
        // The light on a surface, from the empty sites in front of it, not across it.
        let light = self.near(i, x, Some(normal));
        self.on(&light, normal)
    }

    /** Over π, the light on a surface facing `normal` from `light`: its mean over the directions it faces, weighed by their cosines and solid angles. */
    fn on(&self, light: &[Rgb; N], normal: [f64; 3]) -> Rgb {
        let (mut sum, mut total) = ([0.0f64; 3], 0.0);
        for ((unit, solid_angle), light) in self.unit.iter().zip(self.solid_angle).zip(light) {
            let weight = dot(*unit, normal).max(0.0) * solid_angle;
            (0..3).for_each(|c| sum[c] += weight * light[c] as f64);
            total += weight;
        }
        sum.map(|c| (c / total) as f32)
    }

    /** What lights the voxel at `site` (of the base level) to its colour at 1: the light on its surface, which faces its empty neighbours. */
    pub fn on_voxel(&mut self, site: [i64; 3]) -> Rgb {
        let mut normal = [0.0; 3];
        for face in FACES {
            if self.empty(0, add(site, face)) {
                (0..3).for_each(|c| normal[c] += face[c] as f64);
            }
        }
        let light = self.light(0, site);
        if normal == [0.0; 3] {
            // No surface, or all of it: the mean of all its faces'.
            let all = FACES.map(|f| self.on(&light, normalize(f.map(|v| v as f64))));
            return std::array::from_fn(|c| all.iter().map(|l| l[c]).sum::<f32>() / all.len() as f32);
        }
        self.on(&light, normalize(normal))
    }

    /** `picture`, of the base level, lit: each voxel's colour times the light on it, glowing ones' own light added; the sky where it misses. */
    pub fn relight(&mut self, picture: &mut Picture) {
        let sky = self.sky[DIRECTIONS.iter().position(|d| *d == [0, 2, 0]).expect("up")];
        for pixel in 0..picture.material.len() {
            let id = picture.material[pixel] as usize;
            let rgb = if id == 0 {
                sky
            } else {
                let on = self.on_voxel(picture.sites[pixel]);
                std::array::from_fn(|c| self.albedo[id][c] * on[c] + self.emission[id][c])
            };
            for (c, value) in rgb.into_iter().enumerate() {
                picture.rgb[3 * pixel + c] = (255.0 * srgb(value.clamp(0.0, 1.0))).round() as u8;
            }
        }
    }

    /** How many sites' light was found, by level from the base. */
    pub fn found(&self) -> Vec<usize> {
        self.levels.iter().map(|l| l.light.len()).collect()
    }
}

/** An sRGB value as a linear one, and back. */
fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/** Where a site of one level is in the next coarser's units. */
fn half(p: [i64; 3]) -> [f64; 3] {
    p.map(|c| c as f64 / 2.0)
}

/** The solid angles of the directions' shares of the sphere, nearer to each than to the others, from a fine grid of latitudes and longitudes. */
fn solid_angles(unit: &[[f64; 3]; N]) -> [f64; N] {
    let mut angles = [0.0; N];
    let (rows, columns) = (180, 360);
    for row in 0..rows {
        let theta = (row as f64 + 0.5) / rows as f64 * std::f64::consts::PI;
        let area = theta.sin() * (std::f64::consts::PI / rows as f64) * (2.0 * std::f64::consts::PI / columns as f64);
        for column in 0..columns {
            let phi = (column as f64 + 0.5) / columns as f64 * 2.0 * std::f64::consts::PI;
            let x = [theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()];
            let nearest = (0..N).max_by(|&a, &b| dot(unit[a], x).total_cmp(&dot(unit[b], x))).expect("directions");
            angles[nearest] += area;
        }
    }
    angles
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
    use crate::generator::{Field, Generator, KindDef, Layer, Op};

    const STONE: u8 = 0;
    const LAMP: u8 = 1;

    /** A world of stone and white lamps where `at(x, y, z, spacing)` says, between heights ±64. */
    fn world(at: impl Fn(i64, i64, i64, i64) -> Option<u8> + 'static) -> World {
        let kinds = vec![
            KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false },
            KindDef { name: "lamp", kind_type: 6, colors: &["#ffffff"], walk_through: false, glows: true },
        ];
        let layers = vec![Layer::Field(Field::at("all", 0.0, Op::Fill, move |x, y, z, s| at(x, y, z, s).map(|k| (k, None))))];
        World { generator: Generator { kinds, seed: 1, layers }, spawn: [0; 3], heights: (-64, 64), points: Vec::new() }
    }

    /** A site on top of ground below y = 0: at y = −1, x + z odd. */
    fn ground(x: i64, z: i64) -> [i64; 3] {
        assert!((x + z) % 2 != 0, "({x}, {z}) is not on top");
        [x, -1, z]
    }

    #[test]
    fn the_sky_lights_open_ground_and_a_roof_shades_it() {
        let world = world(|x, y, z, s| {
            let roof = x.abs() <= 64 && z.abs() <= 64 && (24..24 + 2 * s).contains(&y);
            (y < 0 || roof).then_some(STONE)
        });
        let mut light = Light::new(&world, 0, Sky { sky: 1.0, sun: 0.0, glow: 0.0 });
        let open = light.on_voxel(ground(401, 400));
        let under = light.on_voxel(ground(1, 0));
        for c in 0..3 {
            assert!((open[c] / SKY[c] - 1.0).abs() < 0.1, "open ground {open:?}, the sky {SKY:?}");
        }
        // Under the middle of the roof, a sky between its edge and the horizon: an eighth of open ground's, the ground's light besides.
        assert!(under[1] < 0.3 * open[1], "under the roof {under:?}, open {open:?}");
    }

    #[test]
    fn walls_keep_the_sky_out() {
        // A hollow box, its walls at least two sites thick at every level.
        let world = world(|x, y, z, s| {
            let wall = 2 * s.max(4);
            let outside = x.abs().max(z.abs()) > 48 || y > 48;
            let inside = x.abs().max(z.abs()) < 48 - wall && y < 48 - wall;
            (y < 0 || !outside && !inside).then_some(STONE)
        });
        let mut light = Light::new(&world, 0, Sky { sky: 1.0, sun: 1.0, glow: 0.0 });
        let inside = light.on_voxel(ground(1, 0));
        assert!(inside.iter().all(|c| *c < 0.02), "inside {inside:?}");
    }

    #[test]
    fn glow_keeps_its_power_at_every_level() {
        let lamps = [[0, 24, 0], [5, 31, 6], [-40, 10, 2], [63, 3, -64], [1, 1, 0]];
        let world = world(move |x, y, z, _| lamps.contains(&[x, y, z]).then_some(LAMP));
        let mut light = Light::new(&world, 0, Sky { sky: 0.0, sun: 0.0, glow: 1.0 });
        light.scan_around(GLOWING - 1, [0; 3]);
        let power: Vec<f64> = (0..GLOWING).map(|i| light.levels[i].glow.values().map(|g| g[0] as f64).sum::<f64>()).collect();
        for p in &power {
            assert!((p / power[0] - 1.0).abs() < 1e-4, "power by level {power:?}");
        }
        assert!((power[0] / (lamps.len() as f64 * (GLOW * SHADOW) as f64) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_lamp_lights_the_ground_as_the_square_of_its_distance() {
        let (height, glow) = (24, 1.0);
        let world = world(move |x, y, z, _| if [x, y, z] == [0, height, 0] { Some(LAMP) } else { (y < 0).then_some(STONE) });
        let mut light = Light::new(&world, 0, Sky { sky: 0.0, sun: 0.0, glow });
        let mut ratios = Vec::new();
        for (x, z) in [(9, 0), (16, 7), (33, 0), (40, 31), (65, 0), (70, 51), (101, 0)] {
            let site = ground(x, z);
            let d = [x as f64, (height - site[1]) as f64, z as f64];
            let d2 = dot(d, d);
            let expected = (GLOW * glow * SHADOW) as f64 * (d[1] / d2.sqrt()) / (std::f64::consts::PI * d2);
            ratios.push(light.on_voxel(site)[0] as f64 / expected);
        }
        // Light from low over the horizon is seen along the nearest of the 18 directions, which may be flatter: more is lost the further.
        assert!(ratios.iter().all(|r| (0.4..2.0).contains(r)), "measured over expected by distance {ratios:?}");
    }
}
