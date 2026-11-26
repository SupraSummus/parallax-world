//! A reference for `Light`, for tests: the light on a voxel's face found
//! by following random rays through the voxels of level 0 (lattice's ray.rs),
//! bounce after bounce till chance ends them, with none of Light's shortcuts (its 18 directions,
//! its levels). Voxels are opaque and reflect evenly (Lambert); the sky is as
//! bright everywhere above the horizon; the sun is a direction, as bright
//! as Light's sun in its one of the 18 (a face weighs each direction by
//! its cosine over their sum, 1 + 2√2). The ground goes on past the box: rays that leave it
//! downwards are taken to meet nothing, which is close enough near the
//! ground.

use super::{ALPHA, Brightness, GLOW, GLOW_ONE, Rgb, SKY, SUN, SUN_DIRECTION};
use lattice::ray::{FACES, first_hit};
use worldgen::World;
use worldgen::generator::generate;
use worldgen::grid::{Box3, COLOUR_ONE, VoxelGrid};

/** What the reference's voxels are generated over: the scenes' and enough ground around them. */
const BOX: Box3 = Box3 { min: [-256, -2, -256], max: [256, 96, 256] };
/** How far a ray goes: across the box. */
const FAR: f64 = 1024.0;
/** How far off a face rays start, so as not to meet it again. */
const OFF: f64 = 1e-6;
/** Bounces always followed; past them, each by chance (Russian roulette), as often as the voxel reflects, at most SURVIVES. */
const BOUNCES: usize = 4;
const SURVIVES: f32 = 0.95;

pub struct Reference {
    grid: VoxelGrid,
    glow: f32,
    sky: Rgb,
    /** The sun's light on a surface facing it, over π, and where it is. */
    sun: Rgb,
    sun_direction: [f64; 3],
}

impl Reference {
    pub fn new(world: &World, brightness: Brightness) -> Reference {
        let sun_direction = normalize(SUN_DIRECTION.map(|v| v as f64));
        Reference {
            grid: generate(&world.generator, &BOX, 0),
            glow: GLOW * brightness.glow / GLOW_ONE as f32,
            sky: SKY.map(|c| c * brightness.sky),
            sun: SUN.map(|c| c * brightness.sun / (1.0 + 2.0 * std::f32::consts::SQRT_2)),
            sun_direction,
        }
    }

    /** What the open faces of the voxel at `site` (level 0) send out on average, over `samples` rays a face; and its standard error, summed over channels. */
    pub fn voxel(&self, [x, y, z]: [i64; 3], samples: u64) -> (Rgb, f32) {
        let open: Vec<usize> =
            (0..FACES.len()).filter(|&f| self.grid.get(x + FACES[f][0], y + FACES[f][1], z + FACES[f][2]) == 0).collect();
        let found: Vec<(Rgb, f32)> = open.iter().map(|&f| self.face([x, y, z], f, samples)).collect();
        let n = open.len().max(1) as f32;
        (
            std::array::from_fn(|c| found.iter().map(|(l, _)| l[c]).sum::<f32>() / n),
            found.iter().map(|(_, e)| e * e).sum::<f32>().sqrt() / n,
        )
    }

    /** The light from face `face` of the voxel at `site` (level 0), as Light::faces gives it, over `samples` rays; and its standard error, summed over channels. */
    fn face(&self, site: [i64; 3], face: usize, samples: u64) -> (Rgb, f32) {
        let (mut sum, mut squares) = ([0.0f64; 3], 0.0f64);
        let mut random = Random(hash(site, face as u64));
        for _ in 0..samples {
            let on = self.on_face(site, face, &mut random, BOUNCES);
            (0..3).for_each(|c| sum[c] += on[c] as f64);
            squares += (on.iter().sum::<f32>() as f64).powi(2);
        }
        let n = samples as f64;
        let mean = sum.map(|c| c / n);
        let variance = squares / n - mean.iter().sum::<f64>().powi(2);
        (mean.map(|c| c as f32), (variance.max(0.0) / n).sqrt() as f32)
    }

    /** One sample of the light from face `face` of the voxel at `site`: at a random point of it, from a random direction, `bounces` more to follow. */
    fn on_face(&self, site: [i64; 3], face: usize, random: &mut Random, bounces: usize) -> Rgb {
        let [x, y, z] = site;
        let ((alpha, colour), _, glow) = self.grid.light(x, y, z, face);
        debug_assert_eq!(alpha, ALPHA, "voxels of the reference's scenes are whole");
        let normal = normalize(FACES[face].map(|v| v as f64));
        let point = add(on_square(site, face, random), normal.map(|c| c * OFF));
        // The sun, if it reaches the point, and one direction's light, as often as its cosine.
        let cosine = dot(normal, self.sun_direction);
        let sun = if cosine > 0.0 && self.hit(point, self.sun_direction).is_none() { cosine as f32 } else { 0.0 };
        let incoming = self.incoming(point, cosine_weighted(normal, random), random, bounces);
        std::array::from_fn(|c| colour[c] as f32 / COLOUR_ONE as f32 * (sun * self.sun[c] + incoming[c]) + glow[c] as f32 * self.glow)
    }

    /** Where a ray from `point` along `direction` first meets a voxel within FAR: how far, its site and the face it enters by. */
    fn hit(&self, point: [f64; 3], direction: [f64; 3]) -> Option<(f64, [i64; 3], usize)> {
        first_hit(point, direction, FAR, |[x, y, z]| self.grid.get(x, y, z) != 0)
    }

    /** One sample of the light coming to `point` from `direction`. */
    fn incoming(&self, point: [f64; 3], direction: [f64; 3], random: &mut Random, bounces: usize) -> Rgb {
        match self.hit(point, direction) {
            None if direction[1] > 0.0 => self.sky,
            None => [0.0; 3],
            Some((_, site, face)) if bounces > 0 => self.on_face(site, face, random, bounces - 1),
            Some((_, [x, y, z], face)) => {
                // Followed as often as it reflects, its light the more for it: on average, all of it.
                let ((_, colour), _, glow) = self.grid.light(x, y, z, face);
                let survives = (colour.iter().copied().max().unwrap_or(0) as f32 / COLOUR_ONE as f32).min(SURVIVES);
                let glow = glow.map(|g| g as f32 * self.glow);
                if random.next() < survives as f64 {
                    let light = self.on_face([x, y, z], face, random, 0);
                    std::array::from_fn(|c| glow[c] + (light[c] - glow[c]) / survives)
                } else {
                    glow
                }
            }
        }
    }
}

/** A random point of face `face` of the voxel at `site`, the cube [site, site + 1): a unit square. */
fn on_square(site: [i64; 3], face: usize, random: &mut Random) -> [f64; 3] {
    let axis = face / 2;
    std::array::from_fn(|k| site[k] as f64 + if k == axis { (face & 1) as f64 } else { random.next() })
}

/** A random direction about `normal`, as often as its cosine with it. */
fn cosine_weighted(normal: [f64; 3], random: &mut Random) -> [f64; 3] {
    let helper = if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let a = normalize(cross(helper, normal));
    let b = cross(normal, a);
    let (r, phi) = (random.next().sqrt(), std::f64::consts::TAU * random.next());
    let up = (1.0 - r * r).sqrt();
    std::array::from_fn(|c| r * phi.cos() * a[c] + r * phi.sin() * b[c] + up * normal[c])
}

/** SplitMix64: random numbers the same on every run. */
struct Random(u64);

impl Random {
    /** In [0, 1). */
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn hash(site: [i64; 3], face: u64) -> u64 {
    site.iter().fold(face, |h, &c| (h.rotate_left(17) ^ c as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|c| c / length)
}
