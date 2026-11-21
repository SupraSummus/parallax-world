//! Each cell of a level held to what is in it at the level FINER_BY
//! finer, one by one: what rays along each of the lattice's 12 directions
//! see of it, inside it only, at both levels. A cell holds what it looks
//! like, not what it is made of: how much of a ray it stops, what that
//! shows and what it glows. The rays are the rows of the finer level's
//! sites through it, each two rays a hair to either side, half each (a
//! row along a face between two cells is both's): nothing drawn by chance
//! but which cells are checked, a sample of those near a surface in each
//! stratum's places, by a hash of where they are. The contract holds
//! levels to it (checks::contract).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fcc::VOXEL_FACES;

use crate::checks::contract::sites;
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::noise::hash64;
use crate::picture::{CROSSING, composited, nearest_site};

/** How many levels finer a level's cells are held to: more costs more, and loses less (a finer cell's mean forgets which of its rays it stops). */
pub const FINER_BY: u32 = 2;

/** Cells checked at each level of a stratum in a region, at most: more costs more, and tells more surely. */
pub const SAMPLES: usize = 256;

/** What rays see of a cell along a direction, a mean over them: how much of them it stops, and what shows (linear: what it stops by its colour, and what it glows). */
pub type Look = [f64; 4];

/** Whether `b`, a box of `level`'s sites, meets any of `places` (lattice units): what a stratum's cells and parts are sampled by. */
pub(crate) fn meets(places: &[Box3], b: &Box3, level: u32) -> bool {
    let b = Box3 { min: b.min.map(|v| v << level), max: b.max.map(|v| v << level) };
    places.iter().any(|place| place.overlaps(&b))
}

/** What rays along a direction see of a cell at the finer level and at its own. */
pub type Cell = [Look; 2];

/** How far a cell's own look is from the finer one's, in the channel they differ the most in. */
pub fn deviation([a, b]: &Cell) -> f64 {
    (0..4).map(|c| (b[c] - a[c]).abs()).fold(0.0, f64::max)
}

/**
 * Of the cells of `level` in `area` near the surfaces of `levels` (the
 * level `against` and its own, each covering `area`), a sample in the
 * places of each of `strata`: those whose voxel reaches into one, each
 * seen along each direction at both levels. By stratum, in order; a cell
 * in the places of several is seen once.
 */
pub(crate) fn cells_of(levels: [&Level; 2], strata: &[(String, Vec<Box3>)], area: &Box3, [against, level]: [u32; 2]) -> Vec<Vec<Cell>> {
    let (s, f) = ((1i64 << level) as f64, (1i64 << against) as f64);
    let within = |c: &[i64; 3]| (0..3).all(|i| c[i] as f64 * s - s >= area.min[i] as f64 && c[i] as f64 * s + s <= area.max[i] as f64);
    let grids = levels.map(|l| &l.grid);
    let near: HashSet<[i64; 3]> = near_surfaces(levels, f / s).filter(within).collect();
    // Ordered by where each is: a stratum's sample is the same in any region holding it.
    let mut near: Vec<(u64, [i64; 3])> = near.into_iter().map(|cell| (hash64(level as u64, cell[0], cell[1], cell[2]), cell)).collect();
    near.sort_unstable();
    // The same rays through every cell, moved to it: its centre is a site of the finer level too.
    let rays: [_; VOXEL_FACES.len()] = std::array::from_fn(|along| rays_through(s, f, along));
    let mut seen: BTreeMap<[i64; 3], Vec<Cell>> = BTreeMap::new();
    let mut see = |cell: [i64; 3]| seen.entry(cell).or_insert_with(|| seen_along(grids, &rays, cell, [f, s])).clone();
    strata
        .iter()
        .map(|(_, places)| {
            // Its voxel reaches a site along each axis.
            let reaches = |c: &&[i64; 3]| meets(places, &Box3 { min: c.map(|v| v - 1), max: c.map(|v| v + 1) }, level);
            let cells: Vec<[i64; 3]> = near.iter().map(|(_, cell)| cell).filter(reaches).take(SAMPLES).copied().collect();
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

/** A level's grid of a region, and its sites at a surface, found once for all it is held against. */
pub(crate) struct Level {
    grid: VoxelGrid,
    /** Each site that stops some light beside one that doesn't stop all of it. */
    surface: Vec<[i32; 3]>,
}

impl Level {
    pub(crate) fn of(grid: VoxelGrid) -> Level {
        let open = |[x, y, z]: [i64; 3]| VOXEL_FACES.iter().any(|f| grid.alpha(x + f[0] as i64, y + f[1] as i64, z + f[2] as i64) < ALPHA);
        let surface = sites(&grid.clip())
            .filter(|&(x, y, z)| grid.alpha(x, y, z) != 0 && open([x, y, z]))
            .map(|(x, y, z)| [x, y, z].map(|v| v as i32))
            .collect();
        Level { grid, surface }
    }
}

/**
 * The cells of a level near a surface, at it or at the level finer
 * (`levels`, the finer's sites `finer` of the level's apart): of each
 * site at a surface, the cells its voxel reaches into (its centre's and
 * its corners' along the axes), each as often as it is reached.
 */
fn near_surfaces<'a>(levels: [&'a Level; 2], finer: f64) -> impl Iterator<Item = [i64; 3]> + 'a {
    levels.into_iter().zip([finer, 1.0]).flat_map(|(level, scale)| {
        level.surface.iter().flat_map(move |site| {
            let at = site.map(|v| v as f64 * scale);
            let corner = move |axis: usize, side: f64| std::array::from_fn(|i| at[i] + if i == axis { side * scale } else { 0.0 });
            [at].into_iter().chain((0..3).flat_map(move |axis| [-0.5, 0.5].map(|side| corner(axis, side)))).map(nearest_site)
        })
    })
}
