//! Each cell of a level held to what is in it at the level FINER_BY
//! finer, one by one: what rays along each of the lattice's 6 directions
//! see of it, inside it only, at both levels. A cell holds what it looks
//! like, not what it is made of: how much of a ray it stops, what that
//! shows and what it glows. At its own level a cell is one site, which
//! each ray crosses whole; at the finer, the rays are the rows of its
//! sites through the cell: nothing drawn by chance but which cells are checked, a sample
//! of those near a surface in each stratum's places, by a hash of where
//! they are. The contract holds levels to it (checks::contract).

use std::collections::BTreeMap;

use lattice::{FastSet, VOXEL_FACES};

use crate::checks::contract::{Held, sites};
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::noise::hash64;
use crate::picture::{Seen, composited, seen_whole};

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
    let within = |b: &Box3, c: &[i64; 3]| (0..3).all(|i| c[i] as f64 * s >= b.min[i] as f64 && (c[i] + 1) as f64 * s <= b.max[i] as f64);
    let grids = levels.map(|l| &l.grid);
    let ours = |c: &[i64; 3]| within(area, c) && !taken.iter().any(|b| within(b, c));
    let near: FastSet<[i64; 3]> = near_surfaces(levels, f / s).collect();
    // Ordered by where each is: a stratum's sample is the same in any region holding it.
    let mut near: Vec<(u64, [i64; 3])> =
        near.into_iter().filter(ours).map(|cell| (hash64(level as u64, cell[0], cell[1], cell[2]), cell)).collect();
    near.sort_unstable();
    // The same rays through every cell, moved to it: its corner is a finer site's corner too.
    let rays: [_; VOXEL_FACES.len()] = std::array::from_fn(|along| rays_through(s, f, along));
    let mut seen: BTreeMap<[i64; 3], Vec<Cell>> = BTreeMap::new();
    let mut see = |cell: [i64; 3]| seen.entry(cell).or_insert_with(|| seen_along(grids, &rays, cell, [f, s])).clone();
    strata
        .iter()
        .map(|stratum| {
            let reaches = |c: &&[i64; 3]| meets(&stratum.places, &Box3 { min: **c, max: c.map(|v| v + 1) }, level);
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
    let corner = cell.map(|v| v as f64 * scales[1]);
    let mut cells = Vec::new();
    for (rays, face) in rays.iter().zip(VOXEL_FACES) {
        let d = face.map(f64::from);
        let mut fine = [0.0; 4];
        for (q, length, weight) in rays {
            let q: [f64; 3] = std::array::from_fn(|c| (corner[c] + q[c]) / scales[0]);
            for (_, seen) in composited(grids[0], q, d, length / scales[0]) {
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
 * The rays along direction `along` through the cell of sites `s` apart
 * with its corner at the origin (lattice units), the cube [0, s)³: one
 * along each row of the sites `f` apart through it, through their
 * middles, entering it a hair inside; each weighed 1.
 */
fn rays_through(s: f64, f: f64, along: usize) -> Vec<Ray> {
    let (axis, hair, n) = (along / 2, 1e-6, (s / f) as i64);
    let across = [(axis + 1) % 3, (axis + 2) % 3];
    let mut rays = Vec::new();
    for (i, j) in (0..n).flat_map(|i| (0..n).map(move |j| (i, j))) {
        let mut p = [0.0; 3];
        (p[across[0]], p[across[1]]) = ((i as f64 + 0.5) * f, (j as f64 + 0.5) * f);
        p[axis] = if VOXEL_FACES[along][axis] > 0 { hair } else { s - hair };
        rays.push((p, s - 2.0 * hair, 1.0));
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
 * site at a surface, the cell its cube is in.
 */
fn near_surfaces<'a>(levels: [&'a Level; 2], finer: f64) -> impl Iterator<Item = [i64; 3]> + 'a {
    levels
        .into_iter()
        .zip([finer, 1.0])
        .flat_map(|(level, scale)| level.surface.iter().map(move |site| site.map(|v| (v as f64 * scale).floor() as i64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * Along every direction a cell's rays are alike, and weighed they fill
     * it: their lengths sum to its volume, s³, over each row's share of
     * the plane across it, f².
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
                let volume = s * s * s / (f * f);
                assert!((filled - volume).abs() < 1e-3, "s {s}, f {f}, along {along}: {filled} for {volume}");
            }
        }
    }
}
