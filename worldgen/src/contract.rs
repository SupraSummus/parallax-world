//! The contract every world keeps (docs/generators.md, "Checks"), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; and each level shows what the level two finer shows.

use crate::generator::{Generator, generate, materials_of};
use crate::grid::{Box3, VoxelGrid};
use crate::noise::Rng;

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

fn nearest_site(p: [f64; 3]) -> [i64; 3] {
    let mut r = p.map(|v| (v + 0.5).floor());
    if (r[0] as i64 + r[1] as i64 + r[2] as i64) & 1 != 0 {
        // Off by one in the coordinate rounded the furthest.
        let errors: Vec<f64> = (0..3).map(|i| (p[i] - r[i]).abs()).collect();
        let k = (0..3).fold(0, |m, i| if errors[i] > errors[m] { i } else { m });
        r[k] += if p[k] > r[k] { 1.0 } else { -1.0 };
    }
    r.map(|v| v as i64)
}

struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
}

/** How far each ray goes before entering a voxel of `grid` (at `level`), and its material type; or till it leaves `region`, type 0. */
fn first_hits(grid: &VoxelGrid, types: &[u8], level: u32, rays: &[Ray], region: &Box3) -> Vec<(f64, u8)> {
    let s = (1u64 << level) as f64;
    let inside = |p: [f64; 3]| (0..3).all(|i| p[i] >= region.min[i] as f64 && p[i] < region.max[i] as f64);
    rays.iter()
        .map(|Ray { origin, direction }| {
            let mut t = 0.0;
            loop {
                let p = [0, 1, 2].map(|i| origin[i] + t * direction[i]);
                if !inside(p) {
                    return (t, 0);
                }
                let [x, y, z] = nearest_site(p.map(|v| v / s));
                let id = grid.get(x, y, z);
                if id != 0 {
                    return (t, types[id as usize]);
                }
                t += 0.1 * s.min(4.0);
            }
        })
        .collect()
}

/** Rays from random points of `region`, in random directions, seeded so failures reproduce. */
fn random_rays(r: &Box3, count: usize) -> Vec<Ray> {
    let mut random = Rng::new(11);
    (0..count)
        .map(|_| {
            let mut origin = [0.0; 3];
            for (i, o) in origin.iter_mut().enumerate() {
                *o = r.min[i] as f64 + random.next() * (r.max[i] - r.min[i]) as f64;
            }
            let z = 2.0 * random.next() - 1.0;
            let a = 2.0 * std::f64::consts::PI * random.next();
            Ray { origin, direction: [(1.0 - z * z).sqrt() * a.cos(), (1.0 - z * z).sqrt() * a.sin(), z] }
        })
        .collect()
}

/**
 * How far a level may stray from the one two finer, in the share of rays
 * through and of materials hit (docs/generators.md).
 */
const TOLERANCE: f64 = 0.1;

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
 * What rays from where a camera could be see of `region` at `level`: as
 * many get through, and they hit the same materials, as at the level two
 * finer, within TOLERANCE. Prints the margins (see them with --nocapture).
 */
pub fn level_shows_what_finer_shows(name: &str, generator: &Generator, region: &Box3, level: u32) {
    let (si, finer) = (1i64 << level, level.saturating_sub(2));
    let s = si as f64;
    let types: Vec<u8> = materials_of(generator).iter().map(|m| m.material_type).collect();
    // At least 16 sites of the level across, around the region's centre.
    let grow = |i: usize| 0.max(8 * si - (region.max[i] - region.min[i]) / 2);
    let area = Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) };
    let reference = generate(generator, &box_at(&area, finer), finer);
    // From where a camera could be: empty sites of the finer level.
    let fs = (1u64 << finer) as f64;
    let rays: Vec<Ray> = random_rays(&area, 4000)
        .into_iter()
        .filter(|ray| {
            let [x, y, z] = nearest_site(ray.origin.map(|v| v / fs));
            reference.get(x, y, z) == 0
        })
        .take(2000)
        .collect();
    let fine = first_hits(&reference, &types, finer, &rays, &area);
    let coarse = first_hits(&generate(generator, &box_at(&area, level), level), &types, level, &rays, &area);
    // A level is seen from afar: of the rays that got 2 of its sites far, how many get 4 more.
    let (from, d) = (2.0 * s, 4.0 * s);
    let through = |hits: &[(f64, u8)]| {
        let seen: Vec<_> = hits.iter().filter(|(t, ty)| *t > from && !(*ty == 0 && *t <= from + d)).collect();
        seen.iter().filter(|(t, _)| *t > from + d).count() as f64 / seen.len() as f64
    };
    let shares = |hits: &[(f64, u8)]| {
        let far: Vec<_> = hits.iter().filter(|(t, _)| *t > from).collect();
        let mut counts = std::collections::BTreeMap::<u8, f64>::new();
        for (_, ty) in &far {
            *counts.entry(*ty).or_insert(0.0) += 1.0 / far.len() as f64;
        }
        counts
    };
    let (a, c) = (shares(&fine), shares(&coarse));
    let keys: std::collections::BTreeSet<u8> = a.keys().chain(c.keys()).copied().collect();
    let tv = keys.iter().map(|t| (a.get(t).unwrap_or(&0.0) - c.get(t).unwrap_or(&0.0)).abs()).sum::<f64>() / 2.0;
    let gap = (through(&coarse) - through(&fine)).abs();
    // The margins, to see with --nocapture.
    println!(
        "{name} level {level}: {:.3} of rays through, {:.3} two levels finer; materials differ by {tv:.3}",
        through(&coarse),
        through(&fine)
    );
    assert!(gap < TOLERANCE, "level {level}: {gap} more or fewer rays get through than at level {finer}");
    assert!(tv < TOLERANCE, "level {level}: the materials hit differ by {tv} from level {finer}");
}

/**
 * The whole contract for `generator`, named `name`, in `region` (one with a
 * bit of everything in it), up to the level `levels` it keeps what a camera
 * sees to. Panics where it breaks it.
 */
pub fn check(name: &str, generator: &Generator, region: Box3, levels: u32) {
    // Up to one level past the checked ones: where things may be drawn as media (a forest's trees).
    for level in [0, 2, levels + 1] {
        whole_equals_parts(generator, &region, level);
    }
    for level in 1..=levels {
        level_shows_what_finer_shows(name, generator, &region, level);
    }
}
