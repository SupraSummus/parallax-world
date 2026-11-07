//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, which hand the light from voxels' faces to the
//! baker (bake's light.rs).
//!
//! Light comes to a site from its 12 neighbours' directions: from one
//! step at the site's level, and beyond it from the next coarser level's
//! light. A surface met sends back its colour times the light on it, also
//! from the coarser level. So each level's light comes from coarser ones
//! only, the coarsest's from the sky.
//!
//! What glows comes from the grids (VoxelGrid::glows): every glowing
//! shape on the sites around its centre, at every level, whether a voxel
//! is kept for it or not, so many weak lamps far away add up. Glow is an
//! intensity (radiance times shadow), and a line of sites stands for an
//! area across it, so glow met along it is spread over that area: as
//! bright as the lamps, on average.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use bake::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use bake::slicing::GRID_LEVELS;

use crate::World;
use crate::generator::{generate, materials_of};
use crate::grid::{Box3, VoxelGrid};
use crate::picture::{FACES, Picture, nearest_site};

type Rgb = [f32; 3];

const N: usize = 12;

/** The directions light comes along: towards the neighbours, through the voxel's faces. */
const DIRECTIONS: [[i64; 3]; N] = FACES;

/** Steps along a direction at a level before the coarser level's light takes over; at the coarsest, before the sky does. */
const STEPS: usize = 1;
const TOP_STEPS: usize = 64;
/** Sites per tile in x and z, at each level, as native's; a tile spans the heights. */
pub const TILE: i64 = 128;
/** What a level keeps before it forgets, to find again if asked: sites' light, tiles (each Light's and their Grids). */
const KEPT_SITES: usize = 1 << 18;
const KEPT_TILES: usize = 16;
const SHARED_TILES: usize = 64;

/** A tile, made once: those that want it while it is made wait. */
type Tile = Arc<OnceLock<VoxelGrid>>;
/** Tiles by level and key. */
type Tiles = Map<(usize, (i64, i64)), Tile>;

/** Tiles of every level, shared by the Lights of one world (a thread's each): generated once, by whichever needs one first. */
#[derive(Clone, Default)]
pub struct Grids(Arc<Mutex<Tiles>>);

impl Grids {
    /** Tile `key` of level `i`, made by `make` if not yet. */
    fn get(&self, i: usize, key: (i64, i64), make: impl FnOnce() -> VoxelGrid) -> Tile {
        let tile = {
            let mut grids = self.0.lock().expect("grids");
            if !grids.contains_key(&(i, key)) && grids.keys().filter(|(l, _)| *l == i).count() >= SHARED_TILES {
                grids.retain(|(l, _), _| *l != i);
            }
            grids.entry((i, key)).or_default().clone()
        };
        tile.get_or_init(make);
        tile
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

/** A level's voxels, generated in tiles as they are needed; the glow of its sites (none: none) and the light at them, as far as found. */
struct Level {
    heights: (i64, i64),
    tiles: Map<(i64, i64), Tile>,
    glow: Map<[i64; 3], Rgb>,
    light: Map<[i64; 3], [Rgb; N]>,
}

pub struct Light {
    world: World,
    grids: Grids,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    unit: [[f64; 3]; N],
    /** Per direction, the face of the voxel it meets that it enters by. */
    enters: [usize; N],
    /** Per material id: its colour as a share of the light on it, and what it gives itself. */
    albedo: Vec<Rgb>,
    emission: Vec<Rgb>,
    /** The sky's radiance per direction (the sun's within it). */
    sky: [Rgb; N],
    /** What the sky looks like, where a picture's rays miss. */
    background: Rgb,
}

impl Light {
    /** The light of `world` under `brightness`, found only where asked, its tiles in `grids`. */
    pub fn new(world: World, brightness: Brightness, grids: Grids) -> Light {
        let Brightness { sky, sun, glow } = brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest)
            .map(|level| Level { heights: world.heights_at(level), tiles: Map::default(), glow: Map::default(), light: Map::default() })
            .collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let enters = DIRECTIONS.map(|d| DIRECTIONS.iter().position(|f| *f == d.map(|c| -c)).expect("faces come in pairs"));
        let materials = materials_of(&world.generator);
        // Light adds up in linear terms; colours are sRGB.
        let colour = |m: &crate::generator::Material| {
            let rgb = m.rgb();
            [16, 8, 0].map(|shift| srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0))
        };
        let albedo = materials.iter().map(colour).collect();
        let emission = materials.iter().map(|m| if m.glows { colour(m).map(|c| c * GLOW * glow) } else { [0.0; 3] }).collect();
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
        Light { world, grids, levels, unit, enters, albedo, emission, sky: skies, background }
    }

    /** Tile `key`'s box at level `i`. */
    fn tile_box(&self, i: usize, key: (i64, i64)) -> Box3 {
        let heights = self.levels[i].heights;
        Box3 { min: [key.0 * TILE, heights.0, key.1 * TILE], max: [(key.0 + 1) * TILE, heights.1, (key.1 + 1) * TILE] }
    }

    /** Keeps tile `key` of level `i`, and the glow at its sites. */
    fn keep(&mut self, i: usize, key: (i64, i64), grid: Tile) {
        let Level { tiles, glow, .. } = &mut self.levels[i];
        for &(site, id, shadow) in &grid.get().expect("made").glows {
            let sum = glow.entry(site).or_default();
            (0..3).for_each(|c| sum[c] += self.emission[id as usize][c] * shadow);
        }
        tiles.insert(key, grid);
    }

    /** The material of face `face` of the voxel at `site` of level `i` (0: empty), its tile generated if not yet. */
    fn face(&mut self, i: usize, [x, y, z]: [i64; 3], face: usize) -> u8 {
        let heights = self.levels[i].heights;
        if y < heights.0 || y >= heights.1 {
            return 0;
        }
        self.hold(i, (x.div_euclid(TILE), z.div_euclid(TILE))).face(x, y, z, face)
    }

    /** Tile `key` of level `i`, from the Grids, generated if not yet. */
    fn hold(&mut self, i: usize, key: (i64, i64)) -> &VoxelGrid {
        if !self.levels[i].tiles.contains_key(&key) {
            let b = self.tile_box(i, key);
            let generator = &self.world.generator;
            let grid = self.grids.get(i, key, || generate(generator, &b, i as u32));
            self.keep(i, key, grid);
        }
        self.levels[i].tiles[&key].get().expect("made")
    }

    fn empty(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.face(i, site, 0) == 0
    }

    /** The light coming to `p` (a site of level `i`) from direction `d`. */
    fn incoming(&mut self, i: usize, p: [i64; 3], d: usize) -> Rgb {
        let coarsest = i + 1 == self.levels.len();
        let (v, heights) = (DIRECTIONS[d], self.levels[i].heights);
        // Lines of sites along a neighbour's direction are √2 apart, half a site per unit of volume: one per √2
        // of area across them at level 0, one per 4^level times that at a level.
        let seen = std::f32::consts::FRAC_1_SQRT_2 / (1u64 << (2 * i)) as f32;
        let (mut q, mut glows) = (p, [0.0; 3]);
        let beyond = 'march: {
            for _ in 0..if coarsest { TOP_STEPS } else { STEPS } {
                let front = q;
                q = add(q, v);
                let met = self.face(i, q, self.enters[d]);
                let glow = self.levels[i].glow.get(&q).copied().unwrap_or_default();
                (0..3).for_each(|c| glows[c] += glow[c] * seen);
                if met != 0 {
                    // The surface met sends back its colour times the light on it, from the coarser level.
                    let facing = self.unit[d].map(|c| -c);
                    let on_it = if coarsest { [0.0; 3] } else { self.near(i + 1, half(front), |l, light| l.on(light, facing)) };
                    break 'march std::array::from_fn(|c| self.albedo[met as usize][c] * on_it[c]);
                }
                // Above every voxel and not going down, or below them and not going up, nothing more is met.
                if q[1] > heights.1 && v[1] >= 0 {
                    break 'march self.sky[d];
                }
                if q[1] < heights.0 - 1 && v[1] <= 0 {
                    break 'march [0.0; 3];
                }
            }
            if coarsest {
                if v[1] >= 0 { self.sky[d] } else { [0.0; 3] }
            } else {
                // From where the coarser level's first step's voxel begins, as this level's last one's ends.
                self.near(i + 1, [0, 1, 2].map(|c| (2 * q[c] - v[c]) as f64 / 4.0), |_, light| light[d])
            }
        };
        std::array::from_fn(|c| glows[c] + beyond[c])
    }

    /** The light coming to site `p` of level `i` from every direction. */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if let Some(light) = self.levels[i].light.get(&p) {
            return *light;
        }
        let mut light = [[0.0; 3]; N];
        for (d, l) in light.iter_mut().enumerate() {
            *l = self.incoming(i, p, d);
        }
        self.levels[i].light.insert(p, light);
        light
    }

    /**
     * What `of` takes from the light coming to point `x` (level `i`'s
     * units), one direction's or that on a surface: its mean over the
     * empty sites `around` x.
     */
    fn near(&mut self, i: usize, x: [f64; 3], of: impl Fn(&Self, &[Rgb; N]) -> Rgb) -> Rgb {
        let (mut sum, mut total) = ([0.0f32; 3], 0.0);
        for (site, weight) in around(x) {
            if self.empty(i, site) {
                let light = self.light(i, site);
                let value = of(self, &light);
                (0..3).for_each(|c| sum[c] += weight as f32 * value[c]);
                total += weight as f32;
            }
        }
        if total > 0.0 { sum.map(|c| c / total) } else { sum }
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
     * Tile `key` of `level` (TILE sites square, the heights tall), lit:
     * the light from each face of each of its voxels with an empty
     * neighbour, its colour times the light on it and what it glows.
     */
    pub fn tile(&mut self, level: u32, key: (i64, i64)) -> VoxelGrid {
        let i = level as usize;
        let mut grid = self.hold(i, key).clone();
        let sites: Vec<[i64; 3]> = grid.sites().collect();
        for site in sites {
            if FACES.iter().all(|f| !self.empty(i, add(site, *f))) {
                continue;
            }
            let light = self.light(i, site);
            let out: FaceLight = std::array::from_fn(|f| {
                let (on, m) = (self.on(&light, self.unit[f]), grid.face(site[0], site[1], site[2], f) as usize);
                std::array::from_fn(|c| self.albedo[m][c] * on[c] + self.emission[m][c])
            });
            grid.set_light(site[0], site[1], site[2], out);
        }
        // Forgets what grew too much, to find it again if asked.
        for level in &mut self.levels {
            if level.light.len() > KEPT_SITES {
                level.light.clear();
            }
            if level.tiles.len() > KEPT_TILES {
                level.tiles.clear();
                level.glow.clear();
            }
        }
        grid
    }

    /** What lights the voxel at `site` of `level` to its colour at 1: the light on its surface, which faces its empty neighbours. */
    pub fn on_voxel(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        let i = level as usize;
        let mut normal = [0.0; 3];
        for face in FACES {
            if self.empty(i, add(site, face)) {
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
                let on = self.on_voxel(level, picture.sites[pixel]);
                std::array::from_fn(|c| self.albedo[id][c] * on[c] + self.emission[id][c])
            };
            for (c, value) in rgb.into_iter().enumerate() {
                picture.rgb[3 * pixel + c] = (255.0 * linear_to_srgb(value.clamp(0.0, 1.0))).round() as u8;
            }
        }
    }
}

/**
 * The sites around point `x` (a level's units), the nearer the more: 1.5
 * less the distance, of the nearest and its neighbours. Glow is shared
 * between them (generate), and light read from them, so that it is not
 * seen only along the lattice's lines through one.
 */
pub(crate) fn around(x: [f64; 3]) -> impl Iterator<Item = ([i64; 3], f64)> {
    let s = nearest_site(x);
    std::iter::once(s).chain(FACES.map(|f| add(s, f))).filter_map(move |site| {
        let r = sub(x, site);
        let weight = 1.5 - dot(r, r).sqrt();
        (weight > 0.0).then_some((site, weight))
    })
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
    use crate::generator::{Far, Field, Generator, KindDef, Layer, Node, Op, Population, SOLID, cuboid};
    use crate::noise::Fx;

    const STONE: u8 = 0;
    const LAMP_KIND: u8 = 1;
    /** Lamps are cubes 4 units wide, as the towns' are: many sites, its glow on those inside. */
    const LAMP: f64 = 4.0;
    /** A lamp's mean shadow: its area over four. */
    const LAMP_SHADOW: f64 = 6.0 * LAMP * LAMP / 4.0;

    /**
     * Stone where `stone(x, y, z, spacing)` says, and white lamps at
     * `lamps`, drawn measured from level 1 on; from y = −64 up to
     * what the coarsest levels' sites hold.
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
            .make(move |_, _| Some(Node::of(shapes.clone()).with_far(Far::measured(Fx::ONE))));
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
        let mut light = Light::new(world, Brightness { sky: 1.0, sun: 0.0, glow: 0.0 }, Grids::default());
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
        let mut light = Light::new(world, Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }, Grids::default());
        let inside = light.on_voxel(0, ground(1, 0));
        assert!(inside.iter().all(|c| *c < 0.02), "inside {inside:?}");
    }

    #[test]
    fn every_level_holds_its_lamps_glow_once() {
        // Measured far away: their voxels are kept by chance, their glow is not.
        let lamps = [[0.0, 24.0, 0.0], [5.0, 31.0, 6.0], [-40.0, 10.0, 2.0], [63.0, 3.0, -64.0], [1.0, 1.0, 0.0]];
        let world = world(&lamps, |_, _, _, _| false);
        for level in 0..=8 {
            // Wide enough to hold the sites a lamp shares its glow with.
            let (r, (y0, y1)) = (2 + (256 >> level), world.heights_at(level));
            // Two boxes side by side: each lamp's glow is in one.
            let halves = [(-r, 0), (0, r)].map(|(x0, x1)| generate(&world.generator, &Box3 { min: [x0, y0, -r], max: [x1, y1, r] }, level));
            let glow: f64 = halves.iter().flat_map(|grid| &grid.glows).map(|g| g.2 as f64).sum();
            assert!((glow - lamps.len() as f64 * LAMP_SHADOW).abs() < 1e-4, "level {level}: {glow}");
        }
    }

    #[test]
    fn a_wall_is_lit_on_the_lamps_side() {
        // A wall one site thick across x, the lamp on its +x side.
        let world = world(&[[80.0, 10.0, 64.0]], |x, y, z, s| y < 0 || (64..64 + s).contains(&x) && y < 32 && (32..96).contains(&z));
        let grid = Light::new(world, Brightness { sky: 0.0, sun: 0.0, glow: 1.0 }, Grids::default()).tile(0, (0, 0));
        let light = grid.into_grid().light(64, 10, 64).expect("the wall is lit");
        let side = |sign: i64| FACES.iter().zip(&light).filter(|(f, _)| f[0] == sign).map(|(_, l)| l[0]).sum::<f32>();
        assert!(side(1) > 100.0 * side(-1), "towards the lamp {}, away {}", side(1), side(-1));
    }

    #[test]
    fn a_lamp_lights_the_ground_as_the_square_of_its_distance() {
        let height = 24.0;
        let world = world(&[[0.0, height, 0.0]], |_, y, _, _| y < 0);
        let mut light = Light::new(world, Brightness { sky: 0.0, sun: 0.0, glow: 1.0 }, Grids::default());
        let mut ratios = Vec::new();
        for (x, z) in [(9, 0), (16, 7), (33, 0), (40, 31), (65, 0), (70, 51), (101, 0)] {
            let d = [x as f64, height + 1.0, z as f64];
            let d2 = dot(d, d);
            let expected = GLOW as f64 * LAMP_SHADOW * (d[1] / d2.sqrt()) / (std::f64::consts::PI * d2);
            ratios.push(light.on_voxel(0, ground(x, z))[0] as f64 / expected);
        }
        // Light from low over the horizon is seen along the nearest of the 12 directions, which may be flatter: more is lost the further.
        assert!(ratios.iter().all(|r| (0.6..1.6).contains(r)), "measured over expected by distance {ratios:?}");
    }
}
