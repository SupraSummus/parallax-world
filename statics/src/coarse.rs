//! Coarse structures from a fine one: level-0 voxels gathered into rigid
//! aggregates (a cell of 2^l·FCC, split into its connected pieces), the fine
//! faces between aggregates kept as they are. The coarse model is the fine
//! one restricted to rigid motions of aggregates: exact in its links,
//! stiffer inside them.

use crate::lattice::*;
use crate::model::*;
use std::collections::HashMap;

/// Nearest site of 2^l·FCC to a level-0 site, ties to the greatest c, b, a.
pub fn coarse_site(p: [i64; 3], l: u32) -> [i64; 3] {
    if l == 0 {
        return p;
    }
    let n = (1i64 << l) as f64;
    let q = [p[0] as f64 / n + 1e-7, p[1] as f64 / n + 2e-7, p[2] as f64 / n + 3e-7];
    let mut r = q.map(f64::round);
    if (r[0] + r[1] + r[2]) as i64 & 1 != 0 {
        let e = [0, 1, 2].map(|i| (q[i] - r[i]).abs());
        let k = (0..3).max_by(|&a, &b| e[a].partial_cmp(&e[b]).unwrap()).unwrap();
        r[k] += if q[k] > r[k] { 1.0 } else { -1.0 };
    }
    let n = n as i64;
    [r[0] as i64 * n, r[1] as i64 * n, r[2] as i64 * n]
}

pub struct Agg {
    pub s: Structure,
    /// Fine node -> aggregate.
    pub of: Vec<usize>,
    /// Fine link -> coarse link (None: inside an aggregate).
    pub link_of: Vec<Option<usize>>,
}

/// Gather fine nodes by `level[k]` (each its own level), then by connected
/// piece within the cell; held nodes apart from free ones.
pub fn aggregate(fine: &Lat, level: &[u32]) -> Agg {
    let n = fine.s.nodes.len();
    let key: Vec<([i64; 3], u32, bool)> = (0..n).map(|k| (coarse_site(fine.site[k], level[k]), level[k], fine.s.nodes[k].fixed)).collect();
    aggregate_by(&fine.s, &key)
}

/// Gather nodes of one key, and connected, into one rigid body.
pub fn aggregate_by<K: PartialEq>(s: &Structure, key: &[K]) -> Agg {
    let n = s.nodes.len();
    // union within a key over live fine links
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut a: usize) -> usize {
        while p[a] != a {
            p[a] = p[p[a]];
            a = p[a];
        }
        a
    }
    for l in s.live() {
        if key[l.i] == key[l.j] {
            let (a, b) = (find(&mut parent, l.i), find(&mut parent, l.j));
            if a != b {
                parent[a] = b;
            }
        }
    }
    let mut id: HashMap<usize, usize> = HashMap::new();
    let mut of = vec![0; n];
    let mut m = Vec::new();
    let mut mx = Vec::new();
    let mut fixed = Vec::new();
    for k in 0..n {
        let r = find(&mut parent, k);
        let a = *id.entry(r).or_insert_with(|| {
            m.push(0.0);
            mx.push([0.0; 3]);
            fixed.push(false);
            m.len() - 1
        });
        of[k] = a;
        let nd = &s.nodes[k];
        m[a] += nd.m;
        mx[a] = add(mx[a], mul(nd.x, nd.m));
        fixed[a] |= nd.fixed;
    }
    let nodes: Vec<Node> = (0..m.len()).map(|a| Node { x: mul(mx[a], 1.0 / m[a]), m: m[a], fixed: fixed[a] }).collect();
    let mut links = Vec::new();
    let mut link_of = vec![None; s.links.len()];
    for (li, l) in s.links.iter().enumerate() {
        let (a, b) = (of[l.i], of[l.j]);
        if a == b || (nodes[a].fixed && nodes[b].fixed) {
            continue;
        }
        let mut c = l.clone();
        c.i = a;
        c.j = b;
        link_of[li] = Some(links.len());
        links.push(c);
    }
    Agg { s: Structure { nodes, links }, of, link_of }
}

/// Fine motions from coarse ones: each fine node moves with its aggregate.
pub fn prolong(fine: &Structure, agg: &Agg, x: &[[f64; 6]]) -> Vec<[f64; 6]> {
    fine.nodes
        .iter()
        .enumerate()
        .map(|(k, nd)| {
            let a = agg.of[k];
            let xa = x[a];
            let r = sub(nd.x, agg.s.nodes[a].x);
            let u = add([xa[0], xa[1], xa[2]], cross([xa[3], xa[4], xa[5]], r));
            [u[0], u[1], u[2], xa[3], xa[4], xa[5]]
        })
        .collect()
}

/// Resultants of fine forces on each aggregate: force, and moment about its centre.
pub fn restrict(fine: &Structure, agg: &Agg, r: &[[f64; 6]]) -> Vec<[f64; 6]> {
    let mut out = vec![[0.0; 6]; agg.s.nodes.len()];
    for (k, nd) in fine.nodes.iter().enumerate() {
        let a = agg.of[k];
        let f = [r[k][0], r[k][1], r[k][2]];
        let m = add([r[k][3], r[k][4], r[k][5]], cross(sub(nd.x, agg.s.nodes[a].x), f));
        for i in 0..3 {
            out[a][i] += f[i];
            out[a][3 + i] += m[i];
        }
    }
    out
}

/// An additive multilevel preconditioner: at every level, the residual's
/// resultants on each aggregate, divided by its block, its rigid motion
/// handed back to the fine voxels. Forces go up as resultants, motions come
/// down as rigid ones.
pub struct Multilevel {
    pub fine_inv: Vec<[[f64; 6]; 6]>,
    pub levels: Vec<(Agg, Vec<[[f64; 6]; 6]>)>,
}

impl Multilevel {
    pub fn new(fine: &Lat, top: u32) -> Self {
        let fine_inv = fine.s.block_inverse();
        let levels = (1..=top)
            .map(|l| {
                let a = aggregate(fine, &vec![l; fine.s.nodes.len()]);
                let inv = a.s.block_inverse();
                (a, inv)
            })
            .collect();
        Multilevel { fine_inv, levels }
    }
    pub fn apply(&self, fine: &Structure, r: &[[f64; 6]], z: &mut [[f64; 6]]) {
        for k in 0..r.len() {
            z[k] = mat6(&self.fine_inv[k], r[k]);
        }
        for (agg, inv) in &self.levels {
            let rc = restrict(fine, agg, r);
            let xc: Vec<[f64; 6]> = rc.iter().zip(inv).map(|(v, m)| mat6(m, *v)).collect();
            let zf = prolong(fine, agg, &xc);
            for k in 0..r.len() {
                for i in 0..6 {
                    z[k][i] += zf[k][i];
                }
            }
        }
    }
}

/// Global–local: each aggregate alone, as a free body under its weight and
/// the forces its boundary faces carry in the coarse solution; the worst
/// utilisation inside it (faces between its own voxels). Returns that per
/// aggregate, and the work (iterations × size) it took.
pub fn local_check(fine: &Structure, agg: &Agg, xc: &[[f64; 6]], skip: &[bool]) -> (Vec<f64>, f64) {
    let xf = prolong(fine, agg, xc);
    let na = agg.s.nodes.len();
    let mut ext = fine.loads();
    for (li, l) in fine.links.iter().enumerate() {
        if l.broken || agg.link_of[li].is_none() {
            continue;
        }
        let (dt, dr) = rel(l, &fine.nodes, &xf);
        let f = mv(&l.kt(), dt);
        let m = mv(&l.kr(), dr);
        for (k, s) in [(l.j, -1.0), (l.i, 1.0)] {
            let r = sub(l.c, fine.nodes[k].x);
            let t = add(cross(r, f), m);
            for a in 0..3 {
                ext[k][a] += s * f[a];
                ext[k][3 + a] += s * t[a];
            }
        }
    }
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); na];
    for k in 0..fine.nodes.len() {
        members[agg.of[k]].push(k);
    }
    let mut inner: Vec<Vec<usize>> = vec![Vec::new(); na];
    for (li, l) in fine.links.iter().enumerate() {
        if !l.broken && agg.link_of[li].is_none() && agg.of[l.i] == agg.of[l.j] {
            inner[agg.of[l.i]].push(li);
        }
    }
    let mut out = vec![0.0; na];
    let mut work = 0.0;
    for a in 0..na {
        if skip[a] || agg.s.nodes[a].fixed || inner[a].is_empty() {
            continue;
        }
        let mem = &members[a];
        let mut idx = std::collections::HashMap::new();
        for (i, &k) in mem.iter().enumerate() {
            idx.insert(k, i);
        }
        let centre = agg.s.nodes[a].x;
        let pin = (0..mem.len())
            .min_by(|&p, &q| norm(sub(fine.nodes[mem[p]].x, centre)).partial_cmp(&norm(sub(fine.nodes[mem[q]].x, centre))).unwrap())
            .unwrap();
        let nodes: Vec<Node> =
            mem.iter().enumerate().map(|(i, &k)| Node { x: fine.nodes[k].x, m: fine.nodes[k].m, fixed: i == pin }).collect();
        let links: Vec<Link> = inner[a]
            .iter()
            .map(|&li| {
                let mut l = fine.links[li].clone();
                l.i = idx[&l.i];
                l.j = idx[&l.j];
                l
            })
            .collect();
        let s = Structure { nodes, links };
        let f: Vec<[f64; 6]> = mem.iter().enumerate().map(|(i, &k)| if i == pin { [0.0; 6] } else { ext[k] }).collect();
        let sol = s.solve(&f, None, 1e-6, 100_000);
        work += sol.iters as f64 * (s.nodes.len() + s.links.len()) as f64;
        out[a] = s.utilisation(&sol.x).into_iter().fold(0.0, f64::max);
    }
    (out, work)
}
