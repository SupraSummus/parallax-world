//! Each cell of a level held to what is in it at the level FINER_BY
//! finer, one by one: what rays along each of the lattice's 12 directions
//! see of it, inside it only, at both levels. A cell holds what it looks
//! like, not what it is made of: how much of a ray it stops, what that
//! shows and what it glows. The rays are the rows of the finer level's
//! sites through it, each two rays a hair to either side, half each (a
//! row along a face between two cells is both's): nothing drawn by chance
//! but which cells are checked, a sample of those near a surface. The
//! contract holds levels to it (checks::contract's `margins`).

use std::collections::BTreeSet;

use fcc::VOXEL_FACES;

use crate::World;
use crate::checks::contract::{TOLERANCE, around, box_at, sites};
use crate::generator::{Generator, generate};
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::noise::Rng;
use crate::picture::{CROSSING, composited, nearest_site};

/** How many levels finer a level's cells are held to: more costs more, and loses less (a finer cell's mean forgets which of its rays it stops). */
pub const FINER_BY: u32 = 2;

/** Cells checked at each level of a region, at most: more costs more, and tells more surely. */
pub const SAMPLES: usize = 2000;

/** What rays see of a cell along a direction, a mean over them: how much of them it stops, and what shows (linear: what it stops by its colour, and what it glows). */
pub type Look = [f64; 4];

/** A cell seen along a direction. */
#[derive(Clone, Debug)]
pub struct Cell {
    /** Its centre, lattice units. */
    pub at: [i64; 3],
    /** The direction, of VOXEL_FACES. */
    pub along: usize,
    /** What rays see of it at the finer level and at its own. */
    pub looks: [Look; 2],
}

impl Cell {
    /** How far its own look is from the finer one's, in the channel they differ the most in. */
    pub fn deviation(&self) -> f64 {
        let [a, b] = self.looks;
        (0..4).map(|c| (b[c] - a[c]).abs()).fold(0.0, f64::max)
    }
}

/** A sample of a level's cells in a region, each seen along each direction, against the level `against`. */
#[derive(Clone, Debug)]
pub struct Cells {
    pub name: String,
    pub region: Box3,
    pub level: u32,
    pub against: u32,
    /** Cells near a surface, those sampled of them. */
    pub near: usize,
    pub cells: Vec<Cell>,
}

impl Cells {
    /** The share of the cells (each along each direction) further than `tolerance` from what is in them. */
    pub fn over(&self, tolerance: f64) -> f64 {
        self.cells.iter().filter(|c| c.deviation() > tolerance).count() as f64 / self.cells.len().max(1) as f64
    }

    /** How far a level's look is from the finer one's on average, channel by channel: what it adds or loses everywhere. */
    pub fn bias(&self) -> Look {
        let n = self.cells.len().max(1) as f64;
        std::array::from_fn(|c| self.cells.iter().map(|cell| cell.looks[1][c] - cell.looks[0][c]).sum::<f64>() / n)
    }
}

/** Upper ends of the buckets a report counts deviations in. */
const BUCKETS: [f64; 7] = [0.01, 0.02, TOLERANCE, 0.1, 0.2, 0.5, 1.0];

impl std::fmt::Display for Cells {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let r = self.region;
        let look = |l: &Look| format!("stops {:.3}, shows {:.3} {:.3} {:.3}", l[0], l[1], l[2], l[3]);
        writeln!(
            f,
            "{} in x {}..{}, y {}..{}, z {}..{}, level {} against {}: {} cells along 12 directions, of {} near a surface",
            self.name,
            r.min[0],
            r.max[0],
            r.min[1],
            r.max[1],
            r.min[2],
            r.max[2],
            self.level,
            self.against,
            self.cells.len() / VOXEL_FACES.len(),
            self.near
        )?;
        let mut counts = [0; BUCKETS.len()];
        for cell in &self.cells {
            counts[BUCKETS.iter().position(|&top| cell.deviation() <= top).unwrap_or(BUCKETS.len() - 1)] += 1;
        }
        let n = self.cells.len().max(1) as f64;
        let buckets: Vec<String> = BUCKETS.iter().zip(counts).map(|(top, k)| format!("≤{top}: {:.1}%", 100.0 * k as f64 / n)).collect();
        writeln!(f, "  apart {}", buckets.join(", "))?;
        writeln!(f, "  over {TOLERANCE} {:.1}%; on average {}", 100.0 * self.over(TOLERANCE), look(&self.bias()))?;
        let mut worst: Vec<&Cell> = self.cells.iter().collect();
        worst.sort_by(|a, b| b.deviation().total_cmp(&a.deviation()));
        for cell in worst.iter().take(3) {
            let [a, b] = cell.looks.map(|l| look(&l));
            writeln!(f, "  at {:?} along {:?}: {a} -> {b}", cell.at, VOXEL_FACES[cell.along])?;
        }
        Ok(())
    }
}

/**
 * A sample of the cells of `level` in `region` of `world`, named `name`,
 * against what is in them: of those near its surfaces, those near the
 * surfaces of `focus` too, if given (a component of it alone, checked in place).
 */
pub(crate) fn cells_of(name: &str, world: &World, focus: Option<&Generator>, region: &Box3, level: u32) -> Cells {
    let against = level.saturating_sub(FINER_BY);
    let (s, f) = ((1i64 << level) as f64, (1i64 << against) as f64);
    let mut area = around(region, level);
    let grow = 0.max((8i64 << level) - (region.max[1] - region.min[1]) / 2);
    (area.min[1], area.max[1]) = (region.min[1] - grow, region.max[1] + grow);
    let grids_of = |generator: &Generator| [against, level].map(|l| generate(generator, &box_at(&area, l), l));
    let grids = grids_of(&world.generator);
    let within = |c: &[i64; 3]| (0..3).all(|i| c[i] as f64 * s - s >= area.min[i] as f64 && c[i] as f64 * s + s <= area.max[i] as f64);
    let mut near = near_surfaces(&grids, f / s);
    if let Some(focus) = focus {
        near = near.intersection(&near_surfaces(&grids_of(focus), f / s)).copied().collect();
    }
    let mut near: Vec<[i64; 3]> = near.into_iter().filter(within).collect();
    let mut random = Rng::new(17);
    for i in (1..near.len()).rev() {
        near.swap(i, (random.next() * (i + 1) as f64) as usize % (i + 1));
    }
    let mut cells = Vec::new();
    // The same rays through every cell, moved to it: its centre is a site of the finer level too.
    let rays: [_; VOXEL_FACES.len()] = std::array::from_fn(|along| rays_through(s, f, along));
    for &cell in near.iter().take(SAMPLES) {
        let centre = cell.map(|v| v as f64 * s);
        for (along, rays) in rays.iter().enumerate() {
            let mut looks = [[0.0; 4]; 2];
            for (q, d, length) in rays {
                let q: [f64; 3] = std::array::from_fn(|c| centre[c] + q[c]);
                // The rays run along the finer sites' rows, through their middles: each crossing stands for CROSSING.
                for (((grid, scale), crossing), look) in grids.iter().zip([f, s]).zip([CROSSING, 1.0]).zip(&mut looks) {
                    for (_, seen) in composited(grid, q.map(|v| v / scale), *d, length / scale, crossing) {
                        look[0] += seen.stopped;
                        (0..3).for_each(|c| look[1 + c] += seen.shown[c] + seen.glow[c]);
                    }
                }
            }
            let looks = looks.map(|l| l.map(|v| v / rays.len().max(1) as f64));
            cells.push(Cell { at: cell.map(|v| v << level), along, looks });
        }
    }
    Cells { name: name.to_string(), region: *region, level, against, near: near.len(), cells }
}

/**
 * The rays along direction `along` through the cell centred at the origin
 * (sites `s` apart, lattice units): those of the rows of the sites `f`
 * apart along it that cross it, each two a hair to either side of its
 * row, each where it enters the cell and how far it goes in it.
 */
fn rays_through(s: f64, f: f64, along: usize) -> Vec<([f64; 3], [f64; 3], f64)> {
    let n = VOXEL_FACES[along].map(|v| v as f64);
    let d = n.map(|v| v / std::f64::consts::SQRT_2);
    // A hair across d, n × (3, 2, 1): along no plane of the lattice.
    let e = [n[1] - 2.0 * n[2], 3.0 * n[2] - n[0], 2.0 * n[0] - 3.0 * n[1]];
    let e = e.map(|v| v * 1e-4 / e.iter().map(|v| v * v).sum::<f64>().sqrt());
    let hair = 1e-6;
    // Each row once, by where it meets the plane across d through the origin.
    let mut rows = BTreeSet::new();
    let reach = (s / f) as i64 + 1;
    for i in -reach..=reach {
        for j in -reach..=reach {
            for k in -reach..=reach {
                if (i + j + k) & 1 != 0 {
                    continue;
                }
                let p = [i, j, k].map(|v| v as f64 * f);
                let along = p[0] * d[0] + p[1] * d[1] + p[2] * d[2];
                rows.insert([0, 1, 2].map(|c| ((p[c] - along * d[c]) * 1e3).round() as i64));
            }
        }
    }
    let mut rays = Vec::new();
    for row in rows {
        for side in [-1.0, 1.0] {
            let q: [f64; 3] = std::array::from_fn(|c| row[c] as f64 / 1e3 + side * e[c]);
            // Where it crosses the cell: |m · x| ≤ s for each face's m.
            let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
            for m in VOXEL_FACES {
                let m = m.map(|v| v as f64);
                let towards = m[0] * d[0] + m[1] * d[1] + m[2] * d[2];
                let off = m[0] * q[0] + m[1] * q[1] + m[2] * q[2];
                if towards.abs() < 1e-12 {
                    if off > s {
                        t1 = f64::NEG_INFINITY;
                    }
                } else if towards > 0.0 {
                    t1 = t1.min((s - off) / towards);
                } else {
                    t0 = t0.max((s - off) / towards);
                }
            }
            if t1 - t0 > 2.0 * hair {
                rays.push((std::array::from_fn(|c| q[c] + (t0 + hair) * d[c]), d, t1 - t0 - 2.0 * hair));
            }
        }
    }
    rays
}

/**
 * The cells of a level near a surface, at it or at the level finer
 * (`grids`, the finer's sites `finer` of the level's apart): of each site
 * that stops some light beside one that doesn't stop all of it, the cells
 * its voxel reaches into (its centre's and its corners' along the axes).
 */
fn near_surfaces(grids: &[VoxelGrid; 2], finer: f64) -> BTreeSet<[i64; 3]> {
    let mut cells = BTreeSet::new();
    for (grid, scale) in grids.iter().zip([finer, 1.0]) {
        for (x, y, z) in sites(&grid.clip()) {
            let open = |f: &[i64; 3]| grid.alpha(x + f[0], y + f[1], z + f[2]) < ALPHA;
            if grid.alpha(x, y, z) == 0 || !VOXEL_FACES.iter().map(|f| f.map(|v| v as i64)).any(|f| open(&f)) {
                continue;
            }
            let at = [x, y, z].map(|v| v as f64 * scale);
            cells.insert(nearest_site(at));
            for axis in 0..3 {
                for side in [-0.5, 0.5] {
                    let mut corner = at;
                    corner[axis] += side * scale;
                    cells.insert(nearest_site(corner));
                }
            }
        }
    }
    cells
}
