//! The contract every world keeps (docs/generators.md, "Checks"), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; each level shows rays from where a camera could be
//! what the level two finer shows; and seen from above, what level 0 shows.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::World;
use crate::generator::{Generator, generate, materials_of};
use crate::grid::{Box3, VoxelGrid, is_site};
use crate::noise::Rng;
use crate::picture::{Camera, FACES, first_voxel, nearest_site, picture};

fn b(min_x: i64, min_y: i64, min_z: i64, max_x: i64, max_y: i64, max_z: i64) -> Box3 {
    Box3 { min: [min_x, min_y, min_z], max: [max_x, max_y, max_z] }
}

/** The box of `level`'s sites covering `region` and a margin of two sites. */
fn box_at(r: &Box3, level: u32) -> Box3 {
    let s = 1i64 << level;
    Box3 { min: r.min.map(|v| v.div_euclid(s) - 2), max: r.max.map(|v| -(-v).div_euclid(s) + 2) }
}

fn sites(b: &Box3) -> impl Iterator<Item = (i64, i64, i64)> + '_ {
    (b.min[2]..b.max[2]).flat_map(move |z| {
        (b.min[1]..b.max[1]).flat_map(move |y| {
            let from = b.min[0] + ((b.min[0] + y + z) & 1);
            (from..b.max[0]).step_by(2).map(move |x| (x, y, z))
        })
    })
}

/**
 * How far a level may stray: from the one two finer, in the share of rays
 * through and of materials hit; from level 0, in the materials seen from
 * above (docs/generators.md).
 */
const TOLERANCE: f64 = 0.1;

/** Rays cast at each level, from as many origins. */
const RAYS: usize = 20_000;

/**
 * Levels from 1 up to this one are seen from above, at most. At level 4,
 * sites 2 m apart, a house's edges, of one kind seen from any side, grow
 * up to half a site and show walls from above, 0.11 off at the valley's
 * town's edge ("Media are isotropic" in docs/generators.md).
 */
const FROM_ABOVE: u32 = 3;

/** A box of `region` at `level` generated in eight parts, alone and in any order, equals it generated whole. */
pub fn whole_equals_parts(generator: &Generator, region: &Box3, level: u32) {
    let whole = box_at(region, level);
    let grid = generate(generator, &whole, level);
    assert!(grid.cells.iter().any(|id| *id != 0), "level {level} is empty");
    // Eight parts, split at random points (seeded, so failures reproduce).
    let mut random = Rng::new(7);
    let mut split = |axis: usize, even: bool| {
        let (lo, hi) = (whole.min[axis] as f64, whole.max[axis] as f64);
        let at = lo + (hi - lo) * (0.2 + 0.6 * random.next());
        (if even { 2.0 * (at / 2.0 + 0.5).floor() } else { (at + 0.5).floor() }) as i64
    };
    let (x, y, z) = (split(0, true), split(1, false), split(2, false));
    let mut parts = Vec::new();
    for (min_x, max_x) in [(whole.min[0], x), (x, whole.max[0])] {
        for (min_y, max_y) in [(whole.min[1], y), (y, whole.max[1])] {
            for (min_z, max_z) in [(whole.min[2], z), (z, whole.max[2])] {
                parts.push(b(min_x, min_y, min_z, max_x, max_y, max_z));
            }
        }
    }
    for part in parts.iter().rev() {
        let alone = generate(generator, part, level);
        for (x, y, z) in sites(part) {
            assert_eq!(alone.get(x, y, z), grid.get(x, y, z), "level {level}: part {part:?} differs at ({x}, {y}, {z})");
        }
    }
}

/**
 * The empty sites of a grid a camera could be at: those it could fly to
 * from above, connected to the top of the grid's box through empty sites
 * (voxels sharing a face). Under the ground's shell, inside a closed
 * house or a boulder are not; under a canopy, open to the sky somewhere,
 * is.
 */
struct Open {
    b: Box3,
    open: Vec<bool>,
}

impl Open {
    fn new(grid: &VoxelGrid) -> Open {
        let b = grid.clip();
        // 0: not reached yet, 1: open, 2: full.
        let mut state = vec![0u8; (0..3).map(|i| b.max[i] - b.min[i]).product::<i64>() as usize];
        let mut stack = Vec::new();
        for z in b.min[2]..b.max[2] {
            for x in b.min[0]..b.max[0] {
                // The top site of the column (x, z): in one of the top two layers.
                let y = b.max[1] - 1 - (x + b.max[1] - 1 + z).rem_euclid(2);
                if y >= b.min[1] && grid.get(x, y, z) == 0 {
                    state[index(&b, [x, y, z])] = 1;
                    stack.push([x, y, z]);
                }
            }
        }
        while let Some(p) = stack.pop() {
            for face in FACES {
                let q = [0, 1, 2].map(|i| p[i] + face[i]);
                if !b.holds(q) {
                    continue;
                }
                let i = index(&b, q);
                if state[i] == 0 {
                    state[i] = if grid.get(q[0], q[1], q[2]) == 0 { 1 } else { 2 };
                    if state[i] == 1 {
                        stack.push(q);
                    }
                }
            }
        }
        Open { b, open: state.into_iter().map(|s| s == 1).collect() }
    }

    fn holds(&self, p: [i64; 3]) -> bool {
        self.b.holds(p) && is_site(p[0], p[1], p[2]) && self.open[index(&self.b, p)]
    }
}

/** Where point p of box b is in an array of b's points, x fastest. */
fn index(b: &Box3, p: [i64; 3]) -> usize {
    let n = [0, 1, 2].map(|i| b.max[i] - b.min[i]);
    ((p[0] - b.min[0]) + n[0] * ((p[1] - b.min[1]) + n[1] * (p[2] - b.min[2]))) as usize
}

struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
    /** How far it goes in the area it is cast in, lattice units. */
    end: f64,
}

/** RAYS rays in random directions from random sites of `area` a camera could be at in `reference` (at `level`), seeded so failures reproduce; and how many origins were tried for them. */
fn rays_from_open(area: &Box3, reference: &VoxelGrid, level: u32) -> (Vec<Ray>, usize) {
    let open = Open::new(reference);
    let s = (1i64 << level) as f64;
    let mut random = Rng::new(11);
    let (mut rays, mut tried) = (Vec::with_capacity(RAYS), 0);
    while rays.len() < RAYS && tried < 50 * RAYS {
        tried += 1;
        let origin: [f64; 3] = std::array::from_fn(|i| area.min[i] as f64 + random.next() * (area.max[i] - area.min[i]) as f64);
        let z = 2.0 * random.next() - 1.0;
        let a = 2.0 * std::f64::consts::PI * random.next();
        let direction = [(1.0 - z * z).sqrt() * a.cos(), (1.0 - z * z).sqrt() * a.sin(), z];
        if !open.holds(nearest_site(origin.map(|v| v / s))) {
            continue;
        }
        // Where it leaves the area.
        let end = (0..3)
            .map(|i| match direction[i] {
                d if d > 0.0 => (area.max[i] as f64 - origin[i]) / d,
                d if d < 0.0 => (area.min[i] as f64 - origin[i]) / d,
                _ => f64::INFINITY,
            })
            .fold(f64::INFINITY, f64::min);
        rays.push(Ray { origin, direction, end });
    }
    (rays, tried)
}

/** How far each ray goes before entering a voxel of `grid` (at `level`), lattice units, and its material type; or till it leaves its area, type 0. */
fn first_hits(grid: &VoxelGrid, types: &[u8], level: u32, rays: &[Ray]) -> Vec<(f64, u8)> {
    let s = (1i64 << level) as f64;
    rays.iter()
        .map(|ray| match first_voxel(grid, ray.origin.map(|v| v / s), ray.direction, ray.end / s) {
            Some((t, id)) => (t * s, types[id as usize]),
            None => (ray.end, 0),
        })
        .collect()
}

/** Half the sum of the shares' differences: how much of one is not in the other. */
fn difference(a: &BTreeMap<u8, f64>, c: &BTreeMap<u8, f64>) -> f64 {
    let keys: BTreeSet<u8> = a.keys().chain(c.keys()).copied().collect();
    keys.iter().map(|t| (a.get(t).unwrap_or(&0.0) - c.get(t).unwrap_or(&0.0)).abs()).sum::<f64>() / 2.0
}

/** Shares of `counts`' keys in their total. */
fn shares(counts: impl IntoIterator<Item = u8>) -> BTreeMap<u8, f64> {
    let mut shares = BTreeMap::new();
    let mut total = 0.0;
    for key in counts {
        *shares.entry(key).or_insert(0.0) += 1.0;
        total += 1.0;
    }
    shares.values_mut().for_each(|v| *v /= total);
    shares
}

/** Shares of a few percent and more, by material type: for messages. */
fn shown(shares: &BTreeMap<u8, f64>) -> String {
    shares.iter().filter(|(_, v)| **v >= 0.02).map(|(t, v)| format!("{t}: {v:.2}")).collect::<Vec<_>>().join(", ")
}

/**
 * What rays from where a camera could be see of `region` at each level
 * from 1 to `levels`: as many get through, and of those that hit
 * something, as many hit each material type, as at the level two finer,
 * within TOLERANCE. Prints the margins (see them with --nocapture);
 * returns where it strays further.
 */
pub fn levels_show_what_finer_shows(name: &str, generator: &Generator, region: &Box3, levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    let types: Vec<u8> = materials_of(generator).iter().map(|m| m.material_type).collect();
    // Grids generated, by level and box: a level's is the reference of the level two coarser.
    let mut grids: Vec<(u32, Box3, VoxelGrid)> = Vec::new();
    let grid = |grids: &mut Vec<(u32, Box3, VoxelGrid)>, level: u32, area: &Box3| -> usize {
        let b = box_at(area, level);
        grids.iter().position(|(l, g, _)| *l == level && *g == b).unwrap_or_else(|| {
            grids.push((level, b, generate(generator, &b, level)));
            grids.len() - 1
        })
    };
    let mut origins: Option<(u32, Box3, Vec<Ray>)> = None;
    for level in 1..=levels {
        let start = Instant::now();
        let (si, finer) = (1i64 << level, level.saturating_sub(2));
        let s = si as f64;
        // At least 16 sites of the level across, around the region's centre.
        let grow = |i: usize| 0.max(8 * si - (region.max[i] - region.min[i]) / 2);
        let area = Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) };
        let reference = grid(&mut grids, finer, &area);
        if origins.as_ref().is_none_or(|(l, a, _)| *l != finer || *a != area) {
            let (rays, tried) = rays_from_open(&area, &grids[reference].2, finer);
            println!("{name} level {level}: rays from {} of {tried} points tried, where a camera could be at level {finer}", rays.len());
            assert!(rays.len() >= RAYS / 2, "level {level}: a camera could be at only {} of {tried} points", rays.len());
            origins = Some((finer, area, rays));
        }
        let rays = &origins.as_ref().expect("made").2;
        let fine = first_hits(&grids[reference].2, &types, finer, rays);
        let coarse_grid = grid(&mut grids, level, &area);
        let coarse = first_hits(&grids[coarse_grid].2, &types, level, rays);
        // A level is seen from afar: of the rays that got 2 of its sites far, how many get 4 more,
        // and what those that stop stop on.
        let (from, d) = (2.0 * s, 4.0 * s);
        let through = |hits: &[(f64, u8)]| {
            let seen: Vec<_> = hits.iter().filter(|(t, ty)| *t > from && !(*ty == 0 && *t <= from + d)).collect();
            seen.iter().filter(|(t, _)| *t > from + d).count() as f64 / seen.len() as f64
        };
        let hit = |hits: &[(f64, u8)]| shares(hits.iter().filter(|(t, ty)| *t > from && *ty != 0).map(|(_, ty)| *ty));
        let (a, c) = (hit(&fine), hit(&coarse));
        let tv = difference(&a, &c);
        let gap = (through(&coarse) - through(&fine)).abs();
        println!(
            "{name} level {level}: {:.3} of rays through, {:.3} two levels finer; materials hit differ by {tv:.3} ({:.1} s)",
            through(&coarse),
            through(&fine),
            start.elapsed().as_secs_f64()
        );
        if gap >= TOLERANCE {
            broken.push(format!("level {level}: {gap:.3} more or fewer rays get through than at level {finer}"));
        }
        if tv >= TOLERANCE {
            broken.push(format!(
                "level {level}: the materials hit differ by {tv:.3} from level {finer}: {} there, {} here",
                shown(&a),
                shown(&c)
            ));
        }
        // Only the grids the coming levels read: the coarse one, and the one after it.
        grids.retain(|(l, ..)| *l + 1 >= level);
    }
    broken
}

/**
 * What a camera straight above `region` sees at each level from 1 to
 * `levels` (at most FROM_ABOVE): the materials of what each ray down
 * hits first, as at level 0, within TOLERANCE. Unlike levels two apart,
 * level 0 catches a drift, a little a level, such as walls taking over
 * from roofs. Prints the margins; returns where it strays further.
 */
pub fn from_above_shows_what_level_0_shows(name: &str, world: &World, region: &Box3, levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    let types: Vec<u8> = materials_of(&world.generator).iter().map(|m| m.material_type).collect();
    // A ray every 2 units.
    let width = ((region.max[0] - region.min[0]) / 2).clamp(16, 1024) as usize;
    let camera = Camera::top([region.min[0] as f64, region.min[2] as f64], [region.max[0] as f64, region.max[2] as f64], width);
    let seen = |level: u32| {
        let start = Instant::now();
        let p = picture(&world.generator, &camera, level, world.heights_at(level));
        (shares(p.material.iter().map(|id| types[*id as usize])), start.elapsed().as_secs_f64())
    };
    let (reference, took) = seen(0);
    println!("{name} from above, level 0: {} ({took:.1} s)", shown(&reference));
    for level in 1..=levels.min(FROM_ABOVE) {
        let (here, took) = seen(level);
        let tv = difference(&reference, &here);
        println!("{name} from above, level {level}: {}; differs by {tv:.3} ({took:.1} s)", shown(&here));
        if tv >= TOLERANCE {
            broken.push(format!(
                "from above, level {level} differs by {tv:.3} from level 0: {} there, {} here",
                shown(&reference),
                shown(&here)
            ));
        }
    }
    broken
}

/**
 * The whole contract for `world`, named `name`, in each of `regions`
 * (between them, a bit of everything in it), up to the level `levels` it
 * keeps what a camera sees to. Panics where it breaks it.
 */
pub fn check(name: &str, world: &World, regions: &[Box3], levels: u32) {
    let mut broken = Vec::new();
    for region in regions {
        let start = Instant::now();
        // Up to one level past the checked ones: where things may be drawn as media (a forest's trees).
        for level in [0, 2, levels + 1] {
            whole_equals_parts(&world.generator, region, level);
        }
        let parts = start.elapsed().as_secs_f64();
        broken.extend(
            levels_show_what_finer_shows(name, &world.generator, region, levels).into_iter().map(|b| format!("in {region:?}, {b}")),
        );
        let rays = start.elapsed().as_secs_f64() - parts;
        broken.extend(from_above_shows_what_level_0_shows(name, world, region, levels).into_iter().map(|b| format!("in {region:?}, {b}")));
        let above = start.elapsed().as_secs_f64() - parts - rays;
        println!("{name} in {region:?}: parts {parts:.1} s, rays {rays:.1} s, from above {above:.1} s");
    }
    assert!(broken.is_empty(), "{name} breaks the contract:\n{}", broken.join("\n"));
}
