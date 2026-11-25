//! Voxels on 2^l·FCC: sites (a, b, c), a + b + c even, at s·(a, b, c);
//! a rhombic dodecahedron each, joined to its 12 neighbours by their faces.

use crate::model::*;
use std::collections::HashMap;

pub const UNIT: f64 = 0.125;

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

pub struct Lattice {
    /// Each voxel's volume.
    pub volume: f64,
    pub voxels: Vec<Voxel>,
    /// `PER_FACE` to a face, one after another.
    pub bonds: Vec<Bond>,
    pub sites: Vec<[i64; 3]>,
}

/// The bonds of the face toward neighbour `e` of a voxel of spacing `s`,
/// centred at `c`: a rhombus, its diagonals √2·s along p and s along q;
/// a quarter of its area at each of four points 1/√12 of a diagonal from
/// the middle, which keeps its second moments.
fn face_bonds(e: [i64; 3], s: f64, c: V3, i: usize, j: usize, mat: Mat) -> [Bond; PER_FACE] {
    let n = unit(e.map(|v| v as f64));
    let mut q = [0.0; 3];
    q[(0..3).find(|&k| e[k] == 0).unwrap()] = 1.0;
    let p = cross(q, n);
    let (dp, dq) = (2f64.sqrt() * s, s);
    let r = 1.0 / 12f64.sqrt();
    let a = dp * dq / 8.0;
    let d = 2f64.sqrt() * s;
    [mul(p, r * dp), mul(p, -r * dp), mul(q, r * dq), mul(q, -r * dq)].map(|o| Bond { i, j, p: add(c, o), n, a, d, mat })
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
                    voxels.push(Voxel { x, m: mat.rho * 2.0 * s * s * s, held });
                    sites.push([a, b, c]);
                    mats.push(mat);
                }
            }
        }
    }
    let mut bonds = Vec::new();
    for (i, site) in sites.iter().enumerate() {
        // Each pair once: by the neighbours whose first coordinate not 0 is positive.
        for e in NEIGHBOURS.iter().filter(|e| e.iter().find(|&&v| v != 0) > Some(&0)) {
            if let Some(&j) = index.get(&[0, 1, 2].map(|k| site[k] + e[k]))
                && !(voxels[i].held && voxels[j].held)
            {
                let c = mul(add(voxels[i].x, voxels[j].x), 0.5);
                bonds.extend(face_bonds(*e, s, c, i, j, mats[i].meet(mats[j])));
            }
        }
    }
    Lattice { volume: 2.0 * s * s * s, voxels, bonds, sites }
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
