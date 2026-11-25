//! Structures on the FCC lattice 2^l·FCC: sites (a,b,c), a+b+c even, at s·(a,b,c).

use crate::model::*;
use std::collections::HashMap;

pub const OFFS: [[i64; 3]; 12] = [
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

/// What fills a point: material, and whether it is held (ground).
pub type Fill<'a> = &'a dyn Fn(V3) -> Option<(Mat, bool)>;

pub struct Lat {
    pub s: Structure,
    pub site: Vec<[i64; 3]>,
    pub index: HashMap<[i64; 3], usize>,
    pub level: u32,
}

/// A face of a voxel of spacing s toward neighbour offset e: its link geometry.
pub fn face(e: [i64; 3], s: f64) -> (V3, V3, V3, f64, f64, f64, f64, Vec<[f64; 2]>) {
    let n = unit([e[0] as f64, e[1] as f64, e[2] as f64]);
    let z = (0..3).find(|&k| e[k] == 0).unwrap();
    let mut q = [0.0; 3];
    q[z] = 1.0;
    let p = cross(q, n);
    let big_p = 2f64.sqrt() * s;
    let big_q = s;
    let a = big_p * big_q / 2.0;
    let ipp = big_q * big_p.powi(3) / 48.0;
    let iqq = big_p * big_q.powi(3) / 48.0;
    let pts = vec![[big_p / 2.0, 0.0], [-big_p / 2.0, 0.0], [0.0, big_q / 2.0], [0.0, -big_q / 2.0]];
    (n, p, q, a, ipp, iqq, 0.0, pts)
}

pub fn weaker(a: Mat, b: Mat) -> Mat {
    Mat {
        name: if a.ft <= b.ft { a.name } else { b.name },
        rho: 0.0,
        e: a.e.min(b.e),
        ft: a.ft.min(b.ft),
        fc: a.fc.min(b.fc),
        fs: a.fs.min(b.fs),
    }
}

/// Point-sampled: each site takes what fills its centre.
pub fn build(level: u32, lo: V3, hi: V3, fill: Fill) -> Lat {
    let s = spacing(level);
    let r = |v: f64, up: bool| if up { (v / s).ceil() as i64 } else { (v / s).floor() as i64 };
    let mut nodes = Vec::new();
    let mut site = Vec::new();
    let mut mats = Vec::new();
    let mut index = HashMap::new();
    for a in r(lo[0], false)..=r(hi[0], true) {
        for b in r(lo[1], false)..=r(hi[1], true) {
            for c in r(lo[2], false)..=r(hi[2], true) {
                if (a + b + c) & 1 != 0 {
                    continue;
                }
                let x = [a as f64 * s, b as f64 * s, c as f64 * s];
                if let Some((m, fixed)) = fill(x) {
                    index.insert([a, b, c], nodes.len());
                    nodes.push(Node { x, m: m.rho * 2.0 * s * s * s, fixed });
                    site.push([a, b, c]);
                    mats.push(m);
                }
            }
        }
    }
    let mut links = Vec::new();
    for (i, st) in site.iter().enumerate() {
        for e in OFFS.iter().take(12) {
            // each pair once: the offset whose first nonzero is positive
            let first = e.iter().find(|&&v| v != 0).unwrap();
            if *first < 0 {
                continue;
            }
            let o = [st[0] + e[0], st[1] + e[1], st[2] + e[2]];
            if let Some(&j) = index.get(&o) {
                if nodes[i].fixed && nodes[j].fixed {
                    continue;
                }
                let (n, p, q, a, ipp, iqq, ipq, pts) = face(*e, s);
                let c = mul(add(nodes[i].x, nodes[j].x), 0.5);
                links.push(Link {
                    i,
                    j,
                    c,
                    n,
                    p,
                    q,
                    d: 2f64.sqrt() * s,
                    a,
                    iqq,
                    ipp,
                    ipq,
                    pts,
                    mat: weaker(mats[i], mats[j]),
                    broken: false,
                });
            }
        }
    }
    Lat { s: Structure { nodes, links }, site, index, level }
}
