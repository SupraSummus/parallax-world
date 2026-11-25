//! A level from the one below, every level by the same rule: each cell of
//! 2^l·FCC keeps, for each pair of its faces, how much its matter joins
//! them; how much weaker that is than it looks; and what it weighs.
//!
//! A level's voxels are nodes on the faces between its cells, each joined
//! to the other faces of the same cell by a beam: one number to a pair of
//! faces, the beam's section, of up to 66. A cell's beams are found from
//! the level below as what moving one of its faces does to the others:
//! each face held as one (a handle) and moved alone along each axis, the
//! matter between free, the handles' reactions are the cell's stiffness
//! between its faces, and the beams' sections those whose stiffness is
//! nearest it. Under those moves the finer matter uses some of its
//! strength, the beams some of theirs: the beams are as much weaker as the
//! finer matter uses more. Matter that only lies near other matter in a
//! cell does not join it, so a truss stays a truss; a cell's weight hangs
//! on the faces its matter reaches, as much as each is joined. Level 0 is
//! voxels and their faces (lattice.rs); every level above, faces and beams.

use crate::bodies::{Work, find, motions, moved};
use crate::lattice::*;
use crate::model::*;
use std::collections::{BTreeMap, HashMap};

/// A beam from a to b of a section `area`, round: the bonds of a patch.
fn beam(i: usize, j: usize, a: V3, b: V3, area: f64, mat: Mat) -> [Bond; PER_FACE] {
    let n = unit(sub(b, a));
    let round = area * area / (4.0 * std::f64::consts::PI);
    let spread: M3 = std::array::from_fn(|r| std::array::from_fn(|c| round * (if r == c { 1.0 } else { 0.0 } - n[r] * n[c])));
    patch(i, j, n, norm(sub(b, a)), area, mul(add(a, b), 0.5), &spread, mat)
}

/// A cell's matter: its free parts, its faces (each named by the cell
/// across), and its bonds between them, each a share of a finer bond.
/// Its places are its free parts first, then a handle to each face.
#[derive(Default)]
struct Cell {
    free: Vec<usize>,
    across: Vec<usize>,
    bonds: Vec<(usize, usize, usize, f64)>,
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
        for &(i, j, _, _) in &self.bonds {
            let (a, b) = (find(&mut parent, i), find(&mut parent, j));
            parent[a] = b;
        }
        (0..self.across.len()).map(|h| find(&mut parent, nf + h)).collect()
    }
}

/// The stiffness of a cell between its handles at `at`, translations only:
/// what holds each handle when one is moved alone along an axis, the
/// others held where they are, the free parts where they come to rest;
/// and the most of its strength the finer matter uses, each move.
fn stiffness(fine: &Lattice, cell: &Cell, at: &[V3]) -> (Vec<Vec<f64>>, Vec<f64>) {
    let (nf, nh) = (cell.free.len(), cell.across.len());
    let mut voxels: Vec<Voxel> = cell.free.iter().map(|&v| Voxel { held: false, ..fine.voxels[v].clone() }).collect();
    voxels.extend(at.iter().map(|&x| Voxel { x, m: 0.0, volume: f64::INFINITY, mat: STONE, held: true }));
    // A handle's site is no body's, as it is held.
    let mut sites: Vec<[i64; 3]> = cell.free.iter().map(|&v| fine.sites[v]).collect();
    sites.resize(nf + nh, [0; 3]);
    let bonds = cell.bonds.iter().map(|&(i, j, b, share)| Bond { i, j, d: fine.bonds[b].d * share, ..fine.bonds[b].clone() }).collect();
    let around = sites.iter().map(|s| [*s, *s]).collect();
    let lat = Lattice { level: fine.level, voxels, bonds, sites, around };
    let alive = vec![true; lat.bonds.len()];
    let gone = vec![false; lat.voxels.len()];
    let mut k = vec![vec![0.0; 3 * nh]; 3 * nh];
    let mut used = Vec::new();
    for h in 0..nh {
        for axis in 0..3 {
            let mut x = vec![[0.0; 6]; nf + nh];
            x[nf + h][axis] = 1.0;
            // What holds the free parts where the handle's move puts them.
            let mut load = vec![[0.0; 6]; nf + nh];
            for (b, f) in lat.bonds.iter().zip(moved(&lat, &x)) {
                for (v, sign) in [(b.j, -1.0), (b.i, 1.0)] {
                    let t = cross(sub(b.p, lat.voxels[v].x), f);
                    for s in 0..3 {
                        load[v][s] += sign * f[s];
                        load[v][3 + s] += sign * t[s];
                    }
                }
            }
            let rest = motions(&lat, &alive, &gone, &|v| load[v], &mut Work(0.0));
            for (xv, r) in x.iter_mut().zip(rest) {
                for s in 0..6 {
                    xv[s] += r[s];
                }
            }
            let force = moved(&lat, &x);
            for (b, f) in lat.bonds.iter().zip(&force) {
                for (v, sign) in [(b.j, 1.0), (b.i, -1.0)] {
                    if v >= nf {
                        for s in 0..3 {
                            k[3 * (v - nf) + s][3 * h + axis] += sign * f[s];
                        }
                    }
                }
            }
            used.push(crate::bodies::used(&lat, &force).into_iter().fold(0.0, f64::max));
        }
    }
    (k, used)
}

/// How much more of its strength the finer matter uses than the beams do,
/// over the moves of each handle along each axis.
fn weakness(found: &[(usize, usize, f64)], at: &[V3], used: &[f64], mat: Mat) -> f64 {
    let (mut fine, mut coarse) = (0.0, 0.0);
    for h in 0..at.len() {
        for axis in 0..3 {
            let mut most: f64 = 0.0;
            for &(a, b, _) in found {
                let (e, length) = (unit(sub(at[b], at[a])), norm(sub(at[b], at[a])));
                let moved = if h == b {
                    e[axis]
                } else if h == a {
                    -e[axis]
                } else {
                    0.0
                };
                let sigma = mat.e * moved / length;
                let s: M3 = std::array::from_fn(|r| std::array::from_fn(|c| sigma * e[r] * e[c]));
                most = most.max(mat.utilisation(&s));
            }
            fine += used[3 * h + axis];
            coarse += most;
        }
    }
    if coarse > 0.0 { fine / coarse } else { 1.0 }
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
        let x = solve(g);
        match idx.iter().zip(&x).filter(|(_, v)| **v < 0.0).min_by(|a, b| a.1.total_cmp(b.1)) {
            Some((&p, _)) => on[p] = false,
            None => return idx.iter().zip(&x).filter(|(_, v)| **v > 0.0).map(|(&p, &v)| (pairs[p].0, pairs[p].1, v)).collect(),
        }
    }
}

/// Gaussian elimination of an augmented matrix; a direction it cannot
/// tell, none.
fn solve(mut a: Vec<Vec<f64>>) -> Vec<f64> {
    let m = a.len();
    let scale = (0..m).map(|r| a[r][r].abs()).fold(0.0, f64::max);
    let small = |v: f64| v.abs() <= 1e-12 * scale;
    for c in 0..m {
        let p = (c..m).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs())).unwrap();
        a.swap(c, p);
        if small(a[c][c]) {
            continue;
        }
        for r in 0..m {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in c..=m {
                    a[r][k] -= f * a[c][k];
                }
            }
        }
    }
    (0..m).map(|r| if small(a[r][r]) { 0.0 } else { a[r][m] / a[r][r] }).collect()
}

/// The next level up.
pub fn gathered(fine: &Lattice) -> Lattice {
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
    // Each fine part: the coarse cells it lies between; each fine bond:
    // those it lies in, one or two.
    let between: Vec<[usize; 2]> = fine.around.iter().map(|a| [cell_of(a[0]), cell_of(a[1])]).collect();
    let lies: Vec<Vec<usize>> = fine
        .bonds
        .iter()
        .map(|b| {
            let mut c: Vec<usize> = fine.cells(b).into_iter().map(&mut cell_of).collect();
            c.dedup();
            c
        })
        .collect();
    let n = sites.len();
    let face = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut cells: Vec<Cell> = (0..n).map(|_| Cell::default()).collect();
    let mut mats: Vec<Vec<(Mat, f64)>> = vec![Vec::new(); n];
    // On each face: what weighs on it, and whether anything on it is held.
    let mut faces: BTreeMap<(usize, usize), (f64, bool)> = BTreeMap::new();
    for (v, &[x, y]) in between.iter().enumerate() {
        let part = &fine.voxels[v];
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
    // A fine part in a cell is itself there; elsewhere, the cell's face
    // toward where it is.
    let mut place = vec![usize::MAX; fine.voxels.len()];
    for cell in &cells {
        for (k, &v) in cell.free.iter().enumerate() {
            place[v] = k;
        }
    }
    let at = |cells: &mut [Cell], c: usize, v: usize| match between[v] {
        [x, y] if x == c && y == c => place[v],
        [x, y] => cells[c].handle(if x == c { y } else { x }),
    };
    // Each fine bond into the cells it lies in, a share of it to each.
    for (b, bond) in fine.bonds.iter().enumerate() {
        for &c in &lies[b] {
            let (i, j) = (at(&mut cells, c, bond.i), at(&mut cells, c, bond.j));
            cells[c].bonds.push((i, j, b, 1.0 / lies[b].len() as f64));
        }
    }
    // A cell is held if all in it is; a face, if anything on it is, or
    // either cell.
    let held: Vec<bool> = cells.iter().map(|c| !c.free.is_empty() && c.free.iter().all(|&v| fine.voxels[v].held)).collect();
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
    let mut voxels: Vec<Voxel> = faces
        .iter()
        .map(|(&(a, b), &(m, on_held))| Voxel {
            x: mul(add(centre(a), centre(b)), 0.5),
            m,
            volume: f64::INFINITY,
            mat: mat_of(&[a, b]),
            held: on_held || held[a] || held[b],
        })
        .collect();
    let mut bonds = Vec::new();
    for (c, cell) in cells.iter().enumerate() {
        let ends: Vec<usize> = cell.across.iter().map(|&o| node[&face(c, o)]).collect();
        let at: Vec<V3> = ends.iter().map(|&k| voxels[k].x).collect();
        let mat = mat_of(&[c]);
        let (found, weaker) = if held[c] || at.is_empty() {
            (Vec::new(), 1.0)
        } else {
            let (k, used) = stiffness(fine, cell, &at);
            let piece = cell.pieces();
            let found = beams(&k, &at, &|a, b| piece[a] == piece[b]);
            let weaker = weakness(&found, &at, &used, mat);
            (found, weaker)
        };
        // The cell's weight on its faces, as much as each is joined.
        let mass: f64 = cell.free.iter().map(|&v| fine.voxels[v].m).sum();
        let mut joined = vec![0.0; at.len()];
        for &(a, b, k) in &found {
            joined[a] += k;
            joined[b] += k;
        }
        let total: f64 = joined.iter().sum();
        for (h, &k) in ends.iter().enumerate() {
            voxels[k].m += mass * if total > 0.0 { joined[h] / total } else { 1.0 / at.len() as f64 };
        }
        // As much weaker as the finer matter uses more of its strength.
        for &(a, b, k) in &found {
            if !(voxels[ends[a]].held && voxels[ends[b]].held) {
                let length = norm(sub(at[b], at[a]));
                bonds.extend(beam(ends[a], ends[b], at[a], at[b], k * length / mat.e, mat.stronger(1.0 / weaker)));
            }
        }
    }
    let around: Vec<[[i64; 3]; 2]> = faces.keys().map(|&(a, b)| [sites[a], sites[b]]).collect();
    Lattice { level, voxels, bonds, sites: around.iter().map(|a| a[0]).collect(), around }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bar of steel, gathered: it weighs as much at every level.
    #[test]
    fn a_bar_weighs_as_much() {
        let fill = |x: V3| (x[0].abs() < 4.0 && x[1].abs() < 0.2 && x[2].abs() < 0.2).then_some((STEEL, false));
        let mut lat = build(0, [-4.5, -1.0, -1.0], [4.5, 1.0, 1.0], &fill);
        let weight = |l: &Lattice| l.voxels.iter().map(|v| v.m).sum::<f64>();
        let whole = weight(&lat);
        for _ in 0..3 {
            lat = gathered(&lat);
            assert!((weight(&lat) / whole - 1.0).abs() < 1e-9, "{} against {whole}", weight(&lat));
        }
    }
}
