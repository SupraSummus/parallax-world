//! Voxels on 2^l·FCC: sites (a, b, c), a + b + c even, at s·(a, b, c);
//! a rhombic dodecahedron each, joined to its 12 neighbours by their faces.

use crate::model::*;
use std::collections::HashMap;

pub const UNIT: f64 = 0.125;

impl Lattice {
    /// The cells of its level a bond lies in: the one its beam is inside,
    /// or the two its face joins.
    pub fn cells(&self, b: &Bond) -> Vec<[i64; 3]> {
        let (a, c) = (self.around[b.i], self.around[b.j]);
        match a.iter().find(|s| c.contains(s)) {
            Some(s) => vec![*s],
            None => vec![a[0], c[0]],
        }
    }
}

/// Bonds to a face.
pub const PER_FACE: usize = 4;

pub const NEIGHBOURS: [[i64; 3]; 12] = [
    [1, 1, 0],
    [1, -1, 0],
    [-1, 1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [1, 0, -1],
    [-1, 0, 1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, 1, -1],
    [0, -1, 1],
    [0, -1, -1],
];

pub fn spacing(level: u32) -> f64 {
    UNIT * (1u64 << level) as f64
}

/// What fills a point: its material, and whether it is held.
pub type Fill<'a> = &'a dyn Fn(V3) -> Option<(Mat, bool)>;

#[derive(Clone)]
pub struct Lattice {
    /// Its voxels are those of 2^level·FCC.
    pub level: u32,
    pub voxels: Vec<Voxel>,
    /// `PER_FACE` to a face, one after another.
    pub bonds: Vec<Bond>,
    /// Each voxel's site, in level-0 units.
    pub sites: Vec<[i64; 3]>,
    /// The cells of its level each voxel lies between: its own, twice, or
    /// for a node on a face, the two the face joins.
    pub around: Vec<[[i64; 3]; 2]>,
}

/// The bonds of a face of normal n between voxels i and j, d apart, of
/// the matter crossing it: its area, its centre, and how it spreads about
/// that centre (∫ s·sᵀ over the area, s across the face). A quarter of
/// the area at each of four points along the spread's axes, as far out as
/// keeps it: so the face bears a moment as the matter would.
#[allow(clippy::too_many_arguments)]
pub fn patch(i: usize, j: usize, n: V3, d: f64, area: f64, centre: V3, spread: &M3, mat: Mat) -> [Bond; PER_FACE] {
    let [(l1, e1), (l2, e2)] = axes(spread, n);
    let r1 = mul(e1, (2.0 * l1 / area).sqrt());
    let r2 = mul(e2, (2.0 * l2 / area).sqrt());
    [r1, mul(r1, -1.0), r2, mul(r2, -1.0)].map(|o| Bond { i, j, p: add(centre, o), n, a: area / 4.0, d, mat })
}

/// The axes of a spread across a face of normal n, and the spread along each.
fn axes(spread: &M3, n: V3) -> [(f64, V3); 2] {
    let a = unit(cross(n, if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
    let b = cross(n, a);
    let along = |u: V3, v: V3| (0..3).map(|r| (0..3).map(|c| u[r] * spread[r][c] * v[c]).sum::<f64>()).sum::<f64>();
    let (saa, sbb, sab) = (along(a, a), along(b, b), along(a, b));
    let angle = 0.5 * (2.0 * sab).atan2(saa - sbb);
    let e1 = add(mul(a, angle.cos()), mul(b, angle.sin()));
    let e2 = cross(n, e1);
    [(along(e1, e1).max(0.0), e1), (along(e2, e2).max(0.0), e2)]
}

/// A face of a voxel of spacing s, toward neighbour e: a rhombus, its
/// diagonals √2·s along p and s along q.
pub fn face(e: [i64; 3], s: f64) -> (V3, f64, M3) {
    let n = unit(e.map(|v| v as f64));
    let mut q = [0.0; 3];
    q[(0..3).find(|&k| e[k] == 0).unwrap()] = 1.0;
    let p = cross(q, n);
    let (dp, dq) = (2f64.sqrt() * s, s);
    let area = dp * dq / 2.0;
    // ∫ s·sᵀ of a rhombus: area·diagonal²/24 along each diagonal.
    let spread = std::array::from_fn(|r| std::array::from_fn(|c| area / 24.0 * (dp * dp * p[r] * p[c] + dq * dq * q[r] * q[c])));
    (n, area, spread)
}

/// Each site takes what fills its centre.
pub fn build(level: u32, lo: V3, hi: V3, fill: Fill) -> Lattice {
    let s = spacing(level);
    let range = |k: usize| (lo[k] / s).floor() as i64..=(hi[k] / s).ceil() as i64;
    let mut voxels = Vec::new();
    let mut sites = Vec::new();
    let mut mats = Vec::new();
    let mut index = HashMap::new();
    for a in range(0) {
        for b in range(1) {
            for c in range(2) {
                if (a + b + c) & 1 != 0 {
                    continue;
                }
                let x = [a, b, c].map(|v| v as f64 * s);
                if let Some((mat, held)) = fill(x) {
                    index.insert([a, b, c], voxels.len());
                    let volume = 2.0 * s * s * s;
                    voxels.push(Voxel { x, m: mat.rho * volume, volume, mat, held });
                    sites.push([a, b, c].map(|v| v << level));
                    mats.push(mat);
                }
            }
        }
    }
    let mut bonds = Vec::new();
    for (i, site) in sites.iter().enumerate() {
        // Each pair once: by the neighbours whose first coordinate not 0 is positive.
        for e in NEIGHBOURS.iter().filter(|e| e.iter().find(|&&v| v != 0) > Some(&0)) {
            if let Some(&j) = index.get(&[0, 1, 2].map(|k| (site[k] >> level) + e[k]))
                && !(voxels[i].held && voxels[j].held)
            {
                let (n, area, spread) = face(*e, s);
                let c = mul(add(voxels[i].x, voxels[j].x), 0.5);
                bonds.extend(patch(i, j, n, 2f64.sqrt() * s, area, c, &spread, mats[i].meet(mats[j])));
            }
        }
    }
    let around = sites.iter().map(|s| [*s, *s]).collect();
    Lattice { level, voxels, bonds, sites, around }
}

/// The site of 2^l·FCC nearest a point in level-0 units, ties to the
/// greatest c, then b, then a.
pub fn coarse_site(p: [i64; 3], l: u32) -> [i64; 3] {
    let n = (1i64 << l) as f64;
    let q = [p[0] as f64 / n + 1e-7, p[1] as f64 / n + 2e-7, p[2] as f64 / n + 3e-7];
    let mut r = q.map(f64::round);
    if (r[0] + r[1] + r[2]) as i64 & 1 != 0 {
        let k = (0..3).max_by(|&a, &b| (q[a] - r[a]).abs().total_cmp(&(q[b] - r[b]).abs())).unwrap();
        r[k] += (q[k] - r[k]).signum();
    }
    r.map(|v| v as i64 * n as i64)
}

/// Hold what lies deep in bulk: a voxel all of whose sites within `radius`
/// are filled is ground that never moves. So a mountain holds what stands
/// on it, and nothing need say where the ground is; bonds between two held
/// voxels go, as they bear nothing.
pub fn hold_bulk(lattice: &mut Lattice, radius: f64) {
    let s = spacing(lattice.level);
    let r = (radius / s).ceil() as i64;
    let near: Vec<[i64; 3]> = (-r..=r)
        .flat_map(|a| (-r..=r).flat_map(move |b| (-r..=r).map(move |c| [a, b, c])))
        .filter(|o| (o[0] + o[1] + o[2]) & 1 == 0 && ((o[0] * o[0] + o[1] * o[1] + o[2] * o[2]) as f64).sqrt() * s <= radius)
        .collect();
    let level = lattice.level;
    let filled: std::collections::HashSet<[i64; 3]> = lattice.sites.iter().map(|p| p.map(|v| v >> level)).collect();
    for (v, site) in lattice.voxels.iter_mut().zip(&lattice.sites) {
        v.held = near.iter().all(|o| filled.contains(&[0, 1, 2].map(|k| (site[k] >> level) + o[k])));
    }
    let voxels = &lattice.voxels;
    lattice.bonds.retain(|b| !(voxels[b.i].held && voxels[b.j].held));
}
