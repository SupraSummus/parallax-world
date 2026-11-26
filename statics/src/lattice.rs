//! Cells of 2^l·Z³, the octree the worlds' levels are (docs/architecture.md):
//! a site p of level l is the cube [2^l p, 2^l (p + 1)), keyed here by
//! its least corner in level-0 units, its 6 faces shared with its
//! neighbours. At every level a node is a face, and a cell joins its
//! faces by beams, 15 to a cell. A level's cells are given (here from
//! what fills space, at level 0; gather.rs measures a coarser level from
//! a finer one): what weighs on each face, and the beams between them,
//! each its section and strength. A cell full of a material has the same
//! beams at every scale (`SECTION`).

use crate::model::*;
use ::lattice::VOXEL_FACES;
use std::collections::{HashMap, HashSet};

/// Metres to a level-0 cell, as the worlds' (worldgen's `UNIT_SIZE`).
pub const UNIT: f64 = 0.125;

/// Bonds to a beam.
pub const PER_BEAM: usize = 4;

/// A full cell's beams: the section of each, over the spacing squared,
/// by the angle between the faces it joins (180°, 90°). A block of full
/// cells strained along an axis, its sides held, is as stiff as its
/// material with these: s² = A₁₈₀ + √2·(1 + G/E)·A₉₀ for stone.
pub const SECTION: [f64; 2] = [0.336, 0.336];

/// The class of a beam between faces at a and b of a cell centred at c:
/// as `SECTION`'s, by the angle between them.
pub fn class(a: V3, b: V3, c: V3) -> usize {
    let (u, v) = (sub(a, c), sub(b, c));
    (1.0 + dot(u, v) / (norm(u) * norm(v))).round() as usize
}

pub fn spacing(level: u32) -> f64 {
    UNIT * (1u64 << level) as f64
}

/// The centre of the cell of level `level` keyed by `site`.
pub fn centre(site: [i64; 3], level: u32) -> V3 {
    site.map(|v| (v as f64 + 0.5 * (1u64 << level) as f64) * UNIT)
}

/// The cell of level l a cell (its key in level-0 units) is in: its
/// parent in the octree, or the parent's parent.
pub fn coarse_site(p: [i64; 3], l: u32) -> [i64; 3] {
    p.map(|v| v >> l << l)
}

/// What fills a point: its material, and whether it is held.
pub type Fill<'a> = &'a dyn Fn(V3) -> Option<(Mat, bool)>;

/// A cell full of a material, its site in level-0 units.
#[derive(Clone, Copy)]
pub struct Solid {
    pub site: [i64; 3],
    pub mat: Mat,
    pub held: bool,
}

#[derive(Clone)]
pub struct Lattice {
    /// Its cells are those of 2^level·Z³.
    pub level: u32,
    pub nodes: Vec<Node>,
    /// `PER_BEAM` to a beam, one after another.
    pub bonds: Vec<Bond>,
    /// The two cells each node lies between, the lesser first (sites in
    /// level-0 units); a node inside a cell of a coarser level is where
    /// two of its finer cells meet.
    pub around: Vec<[[i64; 3]; 2]>,
}

/// A beam between nodes i and j, at a and b, of a round section `area`:
/// a quarter of it at each of four points about its middle, as far out as
/// keeps the section's second moment.
pub fn beam(i: usize, j: usize, a: V3, b: V3, area: f64, mat: Mat) -> [Bond; PER_BEAM] {
    let n = unit(sub(b, a));
    let e1 = unit(cross(n, if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
    let e2 = cross(n, e1);
    // A disc's second moment about a diameter is area²/4π; four points at r keep it.
    let r = (area / (2.0 * std::f64::consts::PI)).sqrt();
    let (middle, d) = (mul(add(a, b), 0.5), norm(sub(b, a)));
    [mul(e1, r), mul(e1, -r), mul(e2, r), mul(e2, -r)].map(|o| Bond { i, j, p: add(middle, o), n, a: area / 4.0, d, mat })
}

/// Each site of level `level` between lo and hi takes what fills its centre.
pub fn solids(level: u32, lo: V3, hi: V3, fill: Fill) -> Vec<Solid> {
    let s = spacing(level);
    let range = |k: usize| (lo[k] / s).floor() as i64..(hi[k] / s).ceil() as i64;
    let mut found = Vec::new();
    for a in range(0) {
        for b in range(1) {
            for c in range(2) {
                let site = [a, b, c].map(|v| v << level);
                if let Some((mat, held)) = fill(centre(site, level)) {
                    found.push(Solid { site, mat, held });
                }
            }
        }
    }
    found
}

/// Hold what lies deep in bulk: a cell all of whose sites within `radius`
/// are filled is ground that never moves. So a mountain holds what stands
/// on it, and nothing need say where the ground is.
pub fn hold_bulk(level: u32, solids: &mut [Solid], radius: f64) {
    let r = (radius / spacing(level)).ceil() as i64;
    let near: Vec<[i64; 3]> = (-r..=r)
        .flat_map(|a| (-r..=r).flat_map(move |b| (-r..=r).map(move |c| [a, b, c])))
        .filter(|o| ((o[0] * o[0] + o[1] * o[1] + o[2] * o[2]) as f64).sqrt() * spacing(level) <= radius)
        .map(|o| o.map(|v| v << level))
        .collect();
    let filled: HashSet<[i64; 3]> = solids.iter().map(|c| c.site).collect();
    let held: Vec<bool> = solids.iter().map(|c| near.iter().all(|o| filled.contains(&[0, 1, 2].map(|k| c.site[k] + o[k])))).collect();
    for (c, h) in solids.iter_mut().zip(held) {
        c.held = h;
    }
}

/// The nodes and beams of full cells: each free cell's faces, its weight
/// shared among them, joined in every pair by a beam of `SECTION`. A face
/// of a held cell is held; held cells need nothing more.
pub fn build(level: u32, solids: &[Solid]) -> Lattice {
    let s = spacing(level);
    let held: HashSet<[i64; 3]> = solids.iter().filter(|c| c.held).map(|c| c.site).collect();
    let mut index: HashMap<[[i64; 3]; 2], usize> = HashMap::new();
    let (mut nodes, mut around, mut bonds) = (Vec::new(), Vec::new(), Vec::new());
    for cell in solids.iter().filter(|c| !c.held) {
        let m = cell.mat.rho * s * s * s / VOXEL_FACES.len() as f64;
        let ends: Vec<usize> = VOXEL_FACES
            .iter()
            .map(|e| {
                let other = [0, 1, 2].map(|k| cell.site[k] + ((e[k] as i64) << level));
                let key = if cell.site < other { [cell.site, other] } else { [other, cell.site] };
                let k = *index.entry(key).or_insert_with(|| {
                    let x = mul(add(centre(cell.site, level), centre(other, level)), 0.5);
                    nodes.push(Node { x, m: 0.0, mat: cell.mat, held: held.contains(&other) });
                    around.push(key);
                    nodes.len() - 1
                });
                let node: &mut Node = &mut nodes[k];
                node.m += m;
                // Of the two cells, the heavier's.
                if cell.mat.rho > node.mat.rho {
                    node.mat = cell.mat;
                }
                k
            })
            .collect();
        let middle = centre(cell.site, level);
        for a in 0..ends.len() {
            for b in a + 1..ends.len() {
                if !(nodes[ends[a]].held && nodes[ends[b]].held) {
                    let (x, y) = (nodes[ends[a]].x, nodes[ends[b]].x);
                    let area = SECTION[class(x, y, middle)] * s * s;
                    bonds.extend(beam(ends[a], ends[b], x, y, area, cell.mat));
                }
            }
        }
    }
    Lattice { level, nodes, bonds, around }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell's parent holds it, and the parent's parent its parent.
    #[test]
    fn cells_nest() {
        for p in [[0, 0, 0], [1, -1, 5], [-3, 7, -8]] {
            for l in 0..4 {
                let up = coarse_site(p, l + 1);
                assert_eq!(coarse_site(coarse_site(p, l), l + 1), up);
                for k in 0..3 {
                    assert!(up[k] <= p[k] && p[k] < up[k] + (1 << (l + 1)), "{p:?} in {up:?} at {}", l + 1);
                }
            }
        }
    }
}
