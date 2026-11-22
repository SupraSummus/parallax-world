//! Each cell of a level held to what is in it at the level FINER_BY
//! finer, one by one: what rays along each of the lattice's 12 directions
//! see of it, inside it only, at both levels. A cell holds what it looks
//! like, not what it is made of: how much of a ray it stops, what that
//! shows and what it glows. At its own level a cell is one site, which
//! each ray crosses whole; at the finer, the rays are the rows of its
//! sites through the cell, a row along a face between two cells half
//! each's: nothing drawn by chance but which cells are checked, a sample
//! of those near a surface in each stratum's places, by a hash of where
//! they are. The contract holds levels to it (checks::contract).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use fcc::{FastSet, VOXEL_FACES};

use crate::checks::contract::{Held, sites};
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::noise::hash64;
use crate::picture::{CROSSING, Seen, composited, nearest_site, seen_whole};

/** How many levels finer a level's cells are held to: more costs more, and loses less (a finer cell's mean forgets which of its rays it stops). */
pub const FINER_BY: u32 = 2;

/** Cells checked at each level of a stratum, at most, shared by the regions holding it in every world checked: more costs more, and tells more surely. */
pub const SAMPLES: usize = 1024;

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
 * Of the cells of `level` in `area` but not in `taken` near the surfaces
 * of `levels` (the level `against` and its own, each covering `area`), a
 * sample in the places of each of `strata`: those whose voxel reaches
 * into one, each seen along each direction at both levels. By stratum, in
 * order; a cell in the places of several is seen once.
 */
pub(crate) fn cells_of(levels: [&Level; 2], strata: &[Held], area: &Box3, taken: &[Box3], [against, level]: [u32; 2]) -> Vec<Vec<Cell>> {
    let (s, f) = ((1i64 << level) as f64, (1i64 << against) as f64);
    let within = |b: &Box3, c: &[i64; 3]| (0..3).all(|i| c[i] as f64 * s - s >= b.min[i] as f64 && c[i] as f64 * s + s <= b.max[i] as f64);
    let grids = levels.map(|l| &l.grid);
    let ours = |c: &[i64; 3]| within(area, c) && !taken.iter().any(|b| within(b, c));
    let near: FastSet<[i64; 3]> = near_surfaces(levels, f / s).collect();
    // Ordered by where each is: a stratum's sample is the same in any region holding it.
    let mut near: Vec<(u64, [i64; 3])> =
        near.into_iter().filter(ours).map(|cell| (hash64(level as u64, cell[0], cell[1], cell[2]), cell)).collect();
    near.sort_unstable();
    // The same rays through every cell, moved to it: its centre is a site of the finer level too.
    let rays: [_; VOXEL_FACES.len()] = std::array::from_fn(|along| rays_through(s, f, along));
    let mut seen: BTreeMap<[i64; 3], Vec<Cell>> = BTreeMap::new();
    let mut see = |cell: [i64; 3]| seen.entry(cell).or_insert_with(|| seen_along(grids, &rays, cell, [f, s])).clone();
    strata
        .iter()
        .map(|stratum| {
            // Its voxel reaches a site along each axis.
            let reaches = |c: &&[i64; 3]| meets(&stratum.places, &Box3 { min: c.map(|v| v - 1), max: c.map(|v| v + 1) }, level);
            let cells: Vec<[i64; 3]> = near.iter().map(|(_, cell)| cell).filter(reaches).take(stratum.of(SAMPLES)).copied().collect();
            cells.into_iter().flat_map(&mut see).collect()
        })
        .collect()
}

/**
 * `cell` seen along each direction by `rays` at the finer level, each as
 * much as it weighs, and at its own (sites `scales` apart, lattice units):
 * there a cell is one site, which each of its rays crosses whole.
 */
fn seen_along(grids: [&VoxelGrid; 2], rays: &[Vec<Ray>], cell: [i64; 3], scales: [f64; 2]) -> Vec<Cell> {
    let centre = cell.map(|v| v as f64 * scales[1]);
    let mut cells = Vec::new();
    for (rays, face) in rays.iter().zip(VOXEL_FACES) {
        let d = face.map(|v| v as f64 / std::f64::consts::SQRT_2);
        let mut fine = [0.0; 4];
        for (q, length, weight) in rays {
            let q: [f64; 3] = std::array::from_fn(|c| (centre[c] + q[c]) / scales[0]);
            // The rays run along the finer sites' rows, through their middles: each crossing stands for CROSSING.
            for (_, seen) in composited(grids[0], q, d, length / scales[0], CROSSING) {
                fine.iter_mut().zip(look(&seen)).for_each(|(sum, v)| *sum += weight * v);
            }
        }
        let weights: f64 = rays.iter().map(|ray| ray.2).sum();
        cells.push([fine.map(|v| v / weights), look(&seen_whole(grids[1], cell, d))]);
    }
    cells
}

/** What a ray sees of a site: how much of it the site stops, and what shows (what it stops by its colour, and what it glows). */
fn look(seen: &Seen) -> Look {
    [seen.stopped, seen.shown[0] + seen.glow[0], seen.shown[1] + seen.glow[1], seen.shown[2] + seen.glow[2]]
}

/** A ray through a cell along a direction: where it enters it, how far it goes in it, and what it weighs. */
type Ray = ([f64; 3], f64, f64);

/**
 * The rays along direction `along` through the cell centred at the origin
 * (sites `s` apart, lattice units): one along each row of the sites `f`
 * apart along it that crosses the cell, entering it a hair inside;
 * weighed 1, or ½ along a face, half each cell's. Only the two faces
 * parallel to it can hold a row, and they are opposite. A row that only
 * touches the cell, at an edge or a corner, crosses none of it.
 */
fn rays_through(s: f64, f: f64, along: usize) -> Vec<Ray> {
    let dot = |a: [i64; 3], b: [i64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let n = VOXEL_FACES[along].map(i64::from);
    let d = n.map(|v| v as f64 / std::f64::consts::SQRT_2);
    let hair = 1e-6;
    // In the finer sites' units, the cell is |m · x| ≤ r for each face's m, and row p is p + t n.
    let r = (s / f) as i64;
    // Each row once, by where it meets the plane across n through the origin (doubled); of
    // one that crosses the cell, the site nearest it is within r + 1 along each axis.
    let mut rows = BTreeMap::new();
    for p in sites(&Box3 { min: [-r - 1; 3], max: [r + 2; 3] }).map(|(x, y, z)| [x, y, z]) {
        rows.entry([0, 1, 2].map(|c| 2 * p[c] - dot(p, n) * n[c])).or_insert(p);
    }
    let mut rays = Vec::new();
    'rows: for p in rows.into_values() {
        let (mut t0, mut t1, mut weight) = (f64::NEG_INFINITY, f64::INFINITY, 1.0);
        for m in VOXEL_FACES.map(|m| m.map(i64::from)) {
            let (towards, off) = (dot(m, n), dot(m, p));
            match towards.cmp(&0) {
                Ordering::Equal if off > r => continue 'rows,
                Ordering::Equal if off == r => weight = 0.5,
                Ordering::Equal => {}
                Ordering::Greater => t1 = t1.min((r - off) as f64 / towards as f64),
                Ordering::Less => t0 = t0.max((r - off) as f64 / towards as f64),
            }
        }
        // Each unit of t is √2 sites of the finer level along the row.
        let length = (t1 - t0) * std::f64::consts::SQRT_2 * f - 2.0 * hair;
        if length > 0.0 {
            rays.push((std::array::from_fn(|c| (p[c] as f64 + t0 * n[c] as f64) * f + hair * d[c]), length, weight));
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

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * Along every direction a cell's rays are alike, and weighed they fill
     * it: their lengths sum to its volume, 2s³, over each row's share of
     * the plane across it, √2 f² (a site's volume, 2 f³, over the rows'
     * spacing, √2 f).
     */
    #[test]
    fn a_cells_rays_fill_it_alike_along_every_direction() {
        for (s, f) in [(2.0, 1.0), (4.0, 1.0), (16.0, 4.0)] {
            let alike = |along: usize| {
                let mut rays: Vec<[i64; 2]> =
                    rays_through(s, f, along).iter().map(|r| [(r.1 * 1e6).round() as i64, (r.2 * 2.0) as i64]).collect();
                rays.sort();
                rays
            };
            for (along, face) in VOXEL_FACES.iter().enumerate() {
                assert_eq!(alike(along), alike(0), "s {s}, f {f}: along {face:?} and {:?}", VOXEL_FACES[0]);
                let filled: f64 = rays_through(s, f, along).iter().map(|r| r.1 * r.2).sum();
                let volume = 2.0 * s * s * s / (std::f64::consts::SQRT_2 * f * f);
                assert!((filled - volume).abs() < 1e-3, "s {s}, f {f}, along {along}: {filled} for {volume}");
            }
        }
    }
}
