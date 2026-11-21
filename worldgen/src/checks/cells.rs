//! Each cell of a level held to what is in it at the level FINER_BY
//! finer, one by one: what rays along each of the lattice's 12 directions
//! see of it, inside it only, at both levels. A cell holds what it looks
//! like, not what it is made of: how much of a ray it stops, what that
//! shows and what it glows. The rays are the rows of the finer level's
//! sites through it, each two rays a hair to either side, half each (a
//! row along a face between two cells is both's): nothing drawn by chance
//! but which cells are checked, a sample of those near a surface in each
//! stratum's places. The contract holds levels to it (checks::contract).

use std::collections::{BTreeMap, BTreeSet};

use fcc::VOXEL_FACES;

use crate::checks::contract::sites;
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::noise::Rng;
use crate::picture::{CROSSING, composited, nearest_site};

/** How many levels finer a level's cells are held to: more costs more, and loses less (a finer cell's mean forgets which of its rays it stops). */
pub const FINER_BY: u32 = 2;

/** Cells checked at each level of a stratum in a region, at most: more costs more, and tells more surely. */
pub const SAMPLES: usize = 256;

/** What rays see of a cell along a direction, a mean over them: how much of them it stops, and what shows (linear: what it stops by its colour, and what it glows). */
pub type Look = [f64; 4];

/** What rays along a direction see of a cell at the finer level and at its own. */
pub type Cell = [Look; 2];

/** How far a cell's own look is from the finer one's, in the channel they differ the most in. */
pub fn deviation([a, b]: &Cell) -> f64 {
    (0..4).map(|c| (b[c] - a[c]).abs()).fold(0.0, f64::max)
}

/**
 * Of the cells of `level` in `area` near the surfaces of `grids` (the
 * level `against` and its own, each covering `area`), a sample in the
 * places of each of `strata`: those whose voxel reaches into one, each
 * seen along each direction at both levels. By stratum, in order; a cell
 * in the places of several is seen once.
 */
pub(crate) fn cells_of(grids: [&VoxelGrid; 2], strata: &[(String, Vec<Box3>)], area: &Box3, [against, level]: [u32; 2]) -> Vec<Vec<Cell>> {
    let (s, f) = ((1i64 << level) as f64, (1i64 << against) as f64);
    let within = |c: &[i64; 3]| (0..3).all(|i| c[i] as f64 * s - s >= area.min[i] as f64 && c[i] as f64 * s + s <= area.max[i] as f64);
    let mut near: Vec<[i64; 3]> = near_surfaces(grids, f / s).into_iter().filter(within).collect();
    let mut random = Rng::new(17);
    for i in (1..near.len()).rev() {
        near.swap(i, (random.next() * (i + 1) as f64) as usize % (i + 1));
    }
    // The same rays through every cell, moved to it: its centre is a site of the finer level too.
    let rays: [_; VOXEL_FACES.len()] = std::array::from_fn(|along| rays_through(s, f, along));
    let mut seen: BTreeMap<[i64; 3], Vec<Cell>> = BTreeMap::new();
    let mut see = |cell: [i64; 3]| seen.entry(cell).or_insert_with(|| seen_along(grids, &rays, cell, [f, s])).clone();
    strata
        .iter()
        .map(|(_, places)| {
            // Its voxel reaches s along each axis.
            let reaches =
                |c: &&[i64; 3]| places.iter().any(|p| (0..3).all(|i| p.min[i] < (c[i] + 1) << level && p.max[i] > (c[i] - 1) << level));
            let cells: Vec<[i64; 3]> = near.iter().filter(reaches).take(SAMPLES).copied().collect();
            cells.into_iter().flat_map(&mut see).collect()
        })
        .collect()
}

/** `cell` seen along each direction by `rays`, at the finer level and its own (sites `scales` apart, lattice units). */
fn seen_along(grids: [&VoxelGrid; 2], rays: &[Vec<Ray>], cell: [i64; 3], scales: [f64; 2]) -> Vec<Cell> {
    let centre = cell.map(|v| v as f64 * scales[1]);
    let mut cells = Vec::new();
    for rays in rays {
        let mut looks = [[0.0; 4]; 2];
        for (q, d, length) in rays {
            let q: [f64; 3] = std::array::from_fn(|c| centre[c] + q[c]);
            // The rays run along the finer sites' rows, through their middles: each crossing stands for CROSSING.
            for (((grid, scale), crossing), look) in grids.iter().zip(scales).zip([CROSSING, 1.0]).zip(&mut looks) {
                for (_, seen) in composited(grid, q.map(|v| v / scale), *d, length / scale, crossing) {
                    look[0] += seen.stopped;
                    (0..3).for_each(|c| look[1 + c] += seen.shown[c] + seen.glow[c]);
                }
            }
        }
        cells.push(looks.map(|l| l.map(|v| v / rays.len().max(1) as f64)));
    }
    cells
}

/** A ray through a cell: where it enters it, its direction and how far it goes in it. */
type Ray = ([f64; 3], [f64; 3], f64);

/**
 * The rays along direction `along` through the cell centred at the origin
 * (sites `s` apart, lattice units): those of the rows of the sites `f`
 * apart along it that cross it, each two a hair to either side of its
 * row, each where it enters the cell and how far it goes in it.
 */
fn rays_through(s: f64, f: f64, along: usize) -> Vec<Ray> {
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
fn near_surfaces(grids: [&VoxelGrid; 2], finer: f64) -> BTreeSet<[i64; 3]> {
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
