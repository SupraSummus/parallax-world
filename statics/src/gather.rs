//! A coarser level measured from a finer one, every level by the same
//! rule: each cell of 2^l·FCC keeps, for each pair of its faces, how much
//! its matter joins them and how strongly, and what it weighs on each.
//!
//! A level's nodes are the faces between its cells, each joined to the
//! other faces of the same cell by a beam: up to 66 to a cell, each its
//! section and its strength. A cell's beams are found from the finer
//! level as what moving one of its faces does to the others: each face
//! held as one (a handle) and moved alone along each axis, the matter
//! between at rest where that leaves it, the handles' reactions are the
//! cell's stiffness between its faces, and the beams' sections those
//! whose stiffness is nearest it. A beam pulled alone, its two faces
//! apart, takes as much of the finer matter's strength as it shows in
//! its own. Matter that only lies near other matter in a cell does not
//! join it, so a truss stays a truss; a cell's weight hangs on the faces
//! its matter reaches, as much as each is joined.

use crate::bodies::{each, find, moved, used};
use crate::lattice::*;
use crate::model::*;
use std::collections::{BTreeMap, HashMap};

/// A cell's matter: its free parts (finer nodes inside it), its faces
/// (each named by the cell across), and the finer bonds between them.
/// Its places are its free parts first, then a handle to each face.
#[derive(Default)]
struct Cell {
    free: Vec<usize>,
    across: Vec<usize>,
    bonds: Vec<(usize, usize, usize)>,
}

impl Cell {
    fn handle(&mut self, other: usize) -> usize {
        self.free.len()
            + self.across.iter().position(|&c| c == other).unwrap_or_else(|| {
                self.across.push(other);
                self.across.len() - 1
            })
    }

    /// Which of its handles its matter joins: a piece of each.
    fn pieces(&self) -> Vec<usize> {
        let nf = self.free.len();
        let mut parent: Vec<usize> = (0..nf + self.across.len()).collect();
        for &(i, j, _) in &self.bonds {
            let (a, b) = (find(&mut parent, i), find(&mut parent, j));
            parent[a] = b;
        }
        (0..self.across.len()).map(|h| find(&mut parent, nf + h)).collect()
    }
}

/// A cell's matter between its handles at `at`, as a lattice of its own
/// (the handles held), and what each handle moved alone along each axis
/// does to it: the finer bonds' forces, handle by handle, axis by axis.
/// The free parts come to rest where the handles leave them: the cell's
/// stiffness condensed onto its handles, by one elimination.
fn responses(fine: &Lattice, cell: &Cell, at: &[V3]) -> (Lattice, Vec<Vec<V3>>) {
    let (nf, nh) = (cell.free.len(), cell.across.len());
    let mut nodes: Vec<Node> = cell.free.iter().map(|&v| Node { held: false, ..fine.nodes[v].clone() }).collect();
    nodes.extend(at.iter().map(|&x| Node { x, m: 0.0, mat: STONE, held: true }));
    let bonds: Vec<Bond> = cell.bonds.iter().map(|&(i, j, b)| Bond { i, j, ..fine.bonds[b].clone() }).collect();
    let lat = Lattice { level: fine.level, nodes, bonds, around: vec![[[0; 3]; 2]; nf + nh] };
    // The free parts' six motions each, then the handles' three; a bond's
    // point moves by the columns of each side.
    let (n, m) = (6 * nf, 3 * nh);
    let column = |v: usize, d: usize, p: V3| -> V3 {
        let mut e = [0.0; 3];
        e[d % 3] = 1.0;
        if d < 3 { e } else { cross(e, sub(p, lat.nodes[v].x)) }
    };
    let dof = |v: usize, d: usize| {
        if v < nf {
            Some(6 * v + d)
        } else if d < 3 {
            Some(n + 3 * (v - nf) + d)
        } else {
            None
        }
    };
    let mut a = vec![vec![0.0; n + m]; n];
    for b in &lat.bonds {
        let s = b.stiffness();
        for (v, sv) in [(b.i, -1.0), (b.j, 1.0)] {
            for d in 0..6 {
                let Some(r) = dof(v, d).filter(|&r| r < n) else { continue };
                let cv = column(v, d, b.p);
                let scv: V3 = std::array::from_fn(|k| dot(s[k], cv));
                for (w, sw) in [(b.i, -1.0), (b.j, 1.0)] {
                    for e in 0..6 {
                        if let Some(c) = dof(w, e) {
                            // The handles' columns go to the right side.
                            let sign = if c < n { 1.0 } else { -1.0 };
                            a[r][c] += sign * sv * sw * dot(scv, column(w, e, b.p));
                        }
                    }
                }
            }
        }
    }
    let x = solve(a, m);
    let cases = (0..m)
        .map(|c| {
            let mut motion = vec![[0.0; 6]; nf + nh];
            for v in 0..nf {
                motion[v] = std::array::from_fn(|d| x[6 * v + d][c]);
            }
            motion[nf + c / 3][c % 3] = 1.0;
            moved(&lat, &motion)
        })
        .collect();
    (lat, cases)
}

/// The beams (a < b, and the stiffness of each along it) whose stiffness
/// is nearest a stiffness between handles at `at`, none less than nothing,
/// and none between handles the cell's matter does not join.
fn beams(k: &[Vec<f64>], at: &[V3], joined: &dyn Fn(usize, usize) -> bool) -> Vec<(usize, usize, f64)> {
    let n = 3 * at.len();
    let pairs: Vec<(usize, usize)> =
        (0..at.len()).flat_map(|a| (a + 1..at.len()).map(move |b| (a, b))).filter(|&(a, b)| joined(a, b)).collect();
    // A unit of each beam's stiffness, as a matrix.
    let units: Vec<Vec<f64>> = pairs
        .iter()
        .map(|&(a, b)| {
            let e = unit(sub(at[b], at[a]));
            let mut d = vec![0.0; n * n];
            for r in 0..3 {
                for c in 0..3 {
                    let v = e[r] * e[c];
                    d[(3 * a + r) * n + 3 * a + c] += v;
                    d[(3 * b + r) * n + 3 * b + c] += v;
                    d[(3 * a + r) * n + 3 * b + c] -= v;
                    d[(3 * b + r) * n + 3 * a + c] -= v;
                }
            }
            d
        })
        .collect();
    let target: Vec<f64> = (0..n * n).map(|i| 0.5 * (k[i / n][i % n] + k[i % n][i / n])).collect();
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    // Least squares, dropping the most negative beam till none is.
    let mut on = vec![true; pairs.len()];
    loop {
        let idx: Vec<usize> = (0..pairs.len()).filter(|&p| on[p]).collect();
        let m = idx.len();
        let mut g = vec![vec![0.0; m + 1]; m];
        for (r, &p) in idx.iter().enumerate() {
            for (c, &q) in idx.iter().enumerate() {
                g[r][c] = dot(&units[p], &units[q]);
            }
            g[r][m] = dot(&units[p], &target);
        }
        let x: Vec<f64> = solve(g, 1).into_iter().map(|r| r[0]).collect();
        match idx.iter().zip(&x).filter(|(_, v)| **v < 0.0).min_by(|a, b| a.1.total_cmp(b.1)) {
            Some((&p, _)) => on[p] = false,
            None => return idx.iter().zip(&x).filter(|(_, v)| **v > 0.0).map(|(&p, &v)| (pairs[p].0, pairs[p].1, v)).collect(),
        }
    }
}

/// Gaussian elimination of a matrix with `rhs` columns to its right; a
/// direction it cannot tell, none. The solutions, row by row.
fn solve(mut a: Vec<Vec<f64>>, rhs: usize) -> Vec<Vec<f64>> {
    let n = a.len();
    let scale = (0..n).map(|r| a[r][r].abs()).fold(0.0, f64::max);
    let small = |v: f64| v.abs() <= 1e-12 * scale;
    for c in 0..n {
        let p = (c..n).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs())).unwrap();
        a.swap(c, p);
        if small(a[c][c]) {
            continue;
        }
        let pivot = a[c].clone();
        for (r, row) in a.iter_mut().enumerate() {
            let f = row[c] / pivot[c];
            if r != c && f != 0.0 {
                for k in c..n + rhs {
                    row[k] -= f * pivot[k];
                }
            }
        }
    }
    (0..n).map(|r| (0..rhs).map(|k| if small(a[r][r]) { 0.0 } else { a[r][n + k] / a[r][r] }).collect()).collect()
}

/// The next level up: each cell's beams measured against a full cell's of
/// its material, class by class, so that a full cell is `SECTION` of its
/// material at every level. Faces held rigid stiffen a cell, and leave
/// the matter where two of them meet carrying all they move; a full cell
/// shows that as much, and is the measure.
pub fn gathered(fine: &Lattice) -> Lattice {
    measured(fine, true)
}

/// A full cell's beams, by class: section over the spacing squared, and
/// how much stronger than its material.
type Full = [(f64, f64); 4];

/// What a full cell of a material measures as, by class: its beams'
/// sections over the spacing squared, and how much stronger than the
/// material they are.
fn full(mat: Mat) -> Full {
    static KNOWN: std::sync::Mutex<Vec<(&'static str, Full)>> = std::sync::Mutex::new(Vec::new());
    if let Some((_, known)) = KNOWN.lock().unwrap().iter().find(|(name, _)| *name == mat.name) {
        return *known;
    }
    let s = spacing(1);
    let coarse = measured(&build(0, &solids(0, [-1.5 * s; 3], [1.5 * s; 3], &|_| Some((mat, false)))), false);
    let mut sum = [(0.0, 0.0, 0.0); 4];
    for b in coarse.bonds.chunks(PER_BEAM) {
        let (i, j) = (b[0].i, b[0].j);
        if coarse.around[i].contains(&[0; 3]) && coarse.around[j].contains(&[0; 3]) {
            let k = class(coarse.nodes[i].x, coarse.nodes[j].x, [0.0; 3]);
            sum[k].0 += 4.0 * b[0].a / (s * s);
            sum[k].1 += b[0].mat.ft / mat.ft;
            sum[k].2 += 1.0;
        }
    }
    let known = sum.map(|(a, f, n)| if n > 0.0 { (a / n, f / n) } else { (1.0, 1.0) });
    KNOWN.lock().unwrap().push((mat.name, known));
    known
}

/// The class of a beam between faces at a and b of a cell centred at c:
/// as `SECTION`'s, by the angle between them.
fn class(a: V3, b: V3, c: V3) -> usize {
    let (u, v) = (sub(a, c), sub(b, c));
    (2.0 + 2.0 * dot(u, v) / (norm(u) * norm(v))).round() as usize
}

/// The next level up, each cell's beams as its matter shows them, or
/// against a full cell's.
fn measured(fine: &Lattice, against_full: bool) -> Lattice {
    let level = fine.level + 1;
    let mut index: HashMap<[i64; 3], usize> = HashMap::new();
    let mut sites: Vec<[i64; 3]> = Vec::new();
    let mut cell_of = |site: [i64; 3]| {
        let key = coarse_site(site, level);
        *index.entry(key).or_insert_with(|| {
            sites.push(key);
            sites.len() - 1
        })
    };
    // Each fine node: the coarse cells it lies between; each fine bond:
    // the one its finer cell is in.
    let between: Vec<[usize; 2]> = fine.around.iter().map(|a| [cell_of(a[0]), cell_of(a[1])]).collect();
    let lies: Vec<usize> = fine
        .bonds
        .iter()
        .map(|b| cell_of(*fine.around[b.i].iter().find(|s| fine.around[b.j].contains(s)).expect("a beam is inside a cell")))
        .collect();
    let n = sites.len();
    let face = |a: usize, b: usize| if sites[a] < sites[b] { (a, b) } else { (b, a) };
    let mut cells: Vec<Cell> = (0..n).map(|_| Cell::default()).collect();
    let mut mats: Vec<Vec<(Mat, f64)>> = vec![Vec::new(); n];
    // On each face: what weighs on it, and whether anything on it is held.
    let mut faces: BTreeMap<(usize, usize), (f64, bool)> = BTreeMap::new();
    for (v, &[x, y]) in between.iter().enumerate() {
        let part = &fine.nodes[v];
        for c in if x == y { vec![x] } else { vec![x, y] } {
            match mats[c].iter_mut().find(|(m, _)| m.name == part.mat.name) {
                Some((_, w)) => *w += part.m,
                None => mats[c].push((part.mat, part.m)),
            }
        }
        if x == y {
            cells[x].free.push(v);
        } else {
            let f = faces.entry(face(x, y)).or_default();
            f.0 += part.m;
            f.1 |= part.held;
        }
    }
    // A fine node in a cell is itself there; on its face, that face.
    let mut place = vec![usize::MAX; fine.nodes.len()];
    for cell in &cells {
        for (k, &v) in cell.free.iter().enumerate() {
            place[v] = k;
        }
    }
    for (b, bond) in fine.bonds.iter().enumerate() {
        let c = lies[b];
        let mut at = |v: usize| match between[v] {
            [x, y] if x == c && y == c => place[v],
            [x, y] => cells[c].handle(if x == c { y } else { x }),
        };
        let (i, j) = (at(bond.i), at(bond.j));
        cells[c].bonds.push((i, j, b));
    }
    // A cell is held if all in it is; a face, if anything on it is, or
    // either cell.
    let held: Vec<bool> = cells.iter().map(|c| !c.free.is_empty() && c.free.iter().all(|&v| fine.nodes[v].held)).collect();
    for (c, cell) in cells.iter().enumerate() {
        for &o in &cell.across {
            faces.entry(face(c, o)).or_default();
        }
    }
    let mat_of = |cs: &[usize]| {
        let mut all: Vec<(Mat, f64)> = Vec::new();
        for &(m, w) in cs.iter().flat_map(|&c| &mats[c]) {
            match all.iter_mut().find(|(a, _)| a.name == m.name) {
                Some((_, x)) => *x += w,
                None => all.push((m, w)),
            }
        }
        all.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map_or(STONE, |m| m.0)
    };
    let centre = |c: usize| sites[c].map(|v| v as f64 * UNIT);
    let node: HashMap<(usize, usize), usize> = faces.keys().enumerate().map(|(k, &f)| (f, k)).collect();
    let mut nodes: Vec<Node> = faces
        .iter()
        .map(|(&(a, b), &(m, on_held))| Node {
            x: mul(add(centre(a), centre(b)), 0.5),
            m,
            mat: mat_of(&[a, b]),
            held: on_held || held[a] || held[b],
        })
        .collect();
    let s = spacing(level);
    let ends: Vec<Vec<usize>> =
        cells.iter().enumerate().map(|(c, cell)| cell.across.iter().map(|&o| node[&face(c, o)]).collect()).collect();
    let at: Vec<Vec<V3>> = ends.iter().map(|e| e.iter().map(|&k| nodes[k].x).collect()).collect();
    let mat: Vec<Mat> = (0..n).map(|c| mat_of(&[c])).collect();
    let fulls: Vec<Full> = mat.iter().map(|&m| if against_full { full(m) } else { [(1.0, 1.0); 4] }).collect();
    // Each cell's beams, all cells at once: its faces a and b, its
    // stiffness along it, how much stronger than its material.
    let found = each(n, |c| {
        let (cell, at, mat, full) = (&cells[c], &at[c], mat[c], &fulls[c]);
        let mut found = Vec::new();
        if !held[c] && !at.is_empty() {
            let (lat, cases) = responses(fine, cell, at);
            // The stiffness between handles: what holds each, each case.
            let nf = cell.free.len();
            let mut k = vec![vec![0.0; cases.len()]; cases.len()];
            for (case, force) in cases.iter().enumerate() {
                for (b, f) in lat.bonds.iter().zip(force) {
                    for (v, sign) in [(b.j, 1.0), (b.i, -1.0)] {
                        if v >= nf {
                            for s in 0..3 {
                                k[3 * (v - nf) + s][case] += sign * f[s];
                            }
                        }
                    }
                }
            }
            let piece = cell.pieces();
            for (a, b, stiff) in beams(&k, at, &|a, b| piece[a] == piece[b]) {
                // The beam pulled alone, a unit apart: the finer matter's
                // forces, and how much of its strength it uses.
                let e = unit(sub(at[b], at[a]));
                let pulled: Vec<V3> = (0..lat.bonds.len())
                    .map(|n| {
                        (0..3)
                            .fold([0.0; 3], |f, s| add(f, add(mul(cases[3 * b + s][n], e[s] / 2.0), mul(cases[3 * a + s][n], -e[s] / 2.0))))
                    })
                    .collect();
                // Each finer bond as much as it holds of the work done:
                // where the matter narrows, most of it.
                let (mut held, mut work) = (0.0, 0.0);
                for ((b, f), u) in lat.bonds.iter().zip(&pulled).zip(used(&lat, &pulled)) {
                    let w = b.energy(*f);
                    held += w * u;
                    work += w;
                }
                let fine_used = if work > 0.0 { held / work } else { 0.0 };
                let length = norm(sub(at[b], at[a]));
                let own = mat.used(mat.e / length, 0.0);
                let stronger = if fine_used > 0.0 { own / fine_used } else { 1.0 };
                let k = class(at[a], at[b], centre(c));
                let section = stiff * length / mat.e / (s * s);
                let section = if against_full { section * SECTION[k] / full[k].0 } else { section };
                found.push((a, b, section * s * s * mat.e / length, stronger / full[k].1));
            }
        }
        found
    });
    let mut bonds = Vec::new();
    for (c, cell) in cells.iter().enumerate() {
        let (found, ends, at, mat) = (&found[c], &ends[c], &at[c], mat[c]);
        // The cell's weight on its faces, as much as each is joined.
        let mass: f64 = cell.free.iter().map(|&v| fine.nodes[v].m).sum();
        let mut joined = vec![0.0; at.len()];
        for &(a, b, k, _) in found {
            joined[a] += k;
            joined[b] += k;
        }
        let total: f64 = joined.iter().sum();
        for (h, &k) in ends.iter().enumerate() {
            nodes[k].m += mass * if total > 0.0 { joined[h] / total } else { 1.0 / at.len() as f64 };
        }
        for &(a, b, k, stronger) in found {
            if !(nodes[ends[a]].held && nodes[ends[b]].held) {
                let length = norm(sub(at[b], at[a]));
                bonds.extend(beam(ends[a], ends[b], at[a], at[b], k * length / mat.e, mat.stronger(stronger)));
            }
        }
    }
    let around = faces.keys().map(|&(a, b)| [sites[a], sites[b]]).collect();
    Lattice { level, nodes, bonds, around }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bar of steel, gathered: it weighs as much at every level.
    #[test]
    fn a_bar_weighs_as_much() {
        let fill = |x: V3| (x[0].abs() < 4.0 && x[1].abs() < 0.2 && x[2].abs() < 0.2).then_some((STEEL, false));
        let mut lat = build(0, &solids(0, [-4.5, -1.0, -1.0], [4.5, 1.0, 1.0], &fill));
        let weight = |l: &Lattice| l.nodes.iter().map(|v| v.m).sum::<f64>();
        let whole = weight(&lat);
        for _ in 0..3 {
            lat = gathered(&lat);
            assert!((weight(&lat) / whole - 1.0).abs() < 1e-9, "{} against {whole}", weight(&lat));
        }
    }
}
