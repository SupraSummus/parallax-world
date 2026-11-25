//! A level from the one below, every level by the same rule: each cell of
//! 2^l·FCC keeps, for each pair of its faces, how much its matter joins
//! them, and nothing else but what it weighs.
//!
//! A level's voxels are nodes on the faces between its cells, each joined
//! to the other faces of the same cell by a beam: one number to a pair of
//! faces, the beam's section, of up to 66. A cell's beams are found from
//! the level below as what moving one of its faces does to the others:
//! each face held as one (a handle) and moved alone along each axis, the
//! matter between free, the handles' reactions are the cell's stiffness
//! between its faces, and the beams' sections those whose stiffness is
//! nearest it. Under those moves the finer matter uses some of its
//! strength, the beams some of theirs: a cell's beams are as much weaker
//! as the finer matter uses more, one number more. So matter that only
//! lies near other matter in a cell does
//! not join it, a truss stays a truss, and a cell's weight hangs on the
//! faces its matter reaches, as much as each is joined. Level 0 is voxels
//! and their faces (lattice.rs); every level above, faces and beams.

use crate::bodies::{Work, motions, stresses};
use crate::lattice::*;
use crate::model::*;
use std::collections::HashMap;

/// A beam from a to b of a section `area`, round: the bonds of a patch.
fn beam(i: usize, j: usize, a: V3, b: V3, area: f64, mat: Mat) -> [Bond; PER_FACE] {
    let n = unit(sub(b, a));
    let round = area * area / (4.0 * std::f64::consts::PI);
    let spread: M3 = std::array::from_fn(|r| std::array::from_fn(|c| round * (if r == c { 1.0 } else { 0.0 } - n[r] * n[c])));
    patch(i, j, n, norm(sub(b, a)), area, mul(add(a, b), 0.5), &spread, mat)
}

/// Where a part of the level below stands in a coarse cell: free inside
/// it, or held with the handle of one of its faces.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Free(usize),
    Handle(usize),
}

/// A cell's matter: its free parts, its faces' handles (each named by the
/// cell across), and its bonds between places, each a share of a finer
/// bond's length.
#[derive(Default)]
struct Cell {
    free: Vec<usize>,
    across: Vec<usize>,
    bonds: Vec<(Place, Place, usize, f64)>,
}

impl Cell {
    fn handle(&mut self, other: usize) -> Place {
        Place::Handle(self.across.iter().position(|&c| c == other).unwrap_or_else(|| {
            self.across.push(other);
            self.across.len() - 1
        }))
    }
}

/// The stiffness of a cell between its handles at `at`, translations only:
/// what holds each handle when one is moved alone along an axis, the
/// others held where they are, the free parts where they come to rest.
fn stiffness(fine: &Lattice, cell: &Cell, at: &[V3]) -> (Vec<Vec<f64>>, Vec<f64>) {
    let (nf, nh) = (cell.free.len(), cell.across.len());
    let mut voxels: Vec<Voxel> = cell.free.iter().map(|&v| Voxel { held: false, ..fine.voxels[v].clone() }).collect();
    let mut sites: Vec<[i64; 3]> = cell.free.iter().map(|&v| fine.sites[v]).collect();
    for &x in at {
        voxels.push(Voxel { x, m: 0.0, volume: f64::INFINITY, mat: STONE, held: true });
        sites.push(sites.first().copied().unwrap_or([0; 3]));
    }
    let index = |p: Place| match p {
        Place::Free(k) => k,
        Place::Handle(h) => nf + h,
    };
    let bonds: Vec<Bond> = cell
        .bonds
        .iter()
        .map(|&(i, j, b, share)| Bond { i: index(i), j: index(j), d: fine.bonds[b].d * share, ..fine.bonds[b].clone() })
        .collect();
    let around = sites.iter().map(|s| [*s, *s]).collect();
    let lat = Lattice { level: fine.level, voxels, bonds, sites, around };
    let alive = vec![true; lat.bonds.len()];
    let gone = vec![false; lat.voxels.len()];
    let point = |m: Six, v: usize, p: V3| -> V3 { add([m[0], m[1], m[2]], cross([m[3], m[4], m[5]], sub(p, lat.voxels[v].x))) };
    let mut k = vec![vec![0.0; 3 * nh]; 3 * nh];
    let mut used = Vec::new();
    for h in 0..nh {
        for axis in 0..3 {
            let mut set = [0.0; 6];
            set[axis] = 1.0;
            let given = |v: usize| if v == nf + h { set } else { [0.0; 6] };
            // What holds the free parts where the handle's move puts them.
            let mut load = vec![[0.0; 6]; lat.voxels.len()];
            for b in &lat.bonds {
                let f = b.force(sub(point(given(b.j), b.j, b.p), point(given(b.i), b.i, b.p)));
                for (v, sign) in [(b.j, -1.0), (b.i, 1.0)] {
                    let t = cross(sub(b.p, lat.voxels[v].x), f);
                    for s in 0..3 {
                        load[v][s] += sign * f[s];
                        load[v][3 + s] += sign * t[s];
                    }
                }
            }
            let rest = motions(&lat, &alive, &gone, &|v| load[v], &mut Work(0.0));
            let moved = |v: usize| -> Six { std::array::from_fn(|s| given(v)[s] + rest[v][s]) };
            let mut force = vec![[0.0; 3]; lat.bonds.len()];
            for (n, b) in lat.bonds.iter().enumerate() {
                let f = b.force(sub(point(moved(b.j), b.j, b.p), point(moved(b.i), b.i, b.p)));
                force[n] = f;
                for (v, sign) in [(b.j, 1.0), (b.i, -1.0)] {
                    if v >= nf {
                        for s in 0..3 {
                            k[3 * (v - nf) + s][3 * h + axis] += sign * f[s];
                        }
                    }
                }
            }
            let s = stresses(&lat, &force);
            let most = lat.bonds.iter().zip(&force).map(|(b, f)| {
                let ends: Vec<usize> = [b.i, b.j].into_iter().filter(|&v| v < nf).collect();
                let mean: M3 = std::array::from_fn(|r| {
                    std::array::from_fn(|c| ends.iter().map(|&v| s[v][r][c]).sum::<f64>() / ends.len().max(1) as f64)
                });
                b.mat.utilisation(&b.stress(&mean, *f))
            });
            used.push(most.fold(0.0, f64::max));
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
/// is nearest a stiffness between handles at `at`, none less than nothing.
fn beams(k: &[Vec<f64>], at: &[V3]) -> Vec<(usize, usize, f64)> {
    let n = 3 * at.len();
    let pairs: Vec<(usize, usize)> = (0..at.len()).flat_map(|a| (a + 1..at.len()).map(move |b| (a, b))).collect();
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

/// The nodes of a level: one on each face something reaches.
struct Nodes {
    at: HashMap<(usize, usize), usize>,
    voxels: Vec<Voxel>,
    around: Vec<[[i64; 3]; 2]>,
}

impl Nodes {
    fn on(&mut self, a: usize, b: usize, sites: &[[i64; 3]], held: &[bool], mat: Mat) -> usize {
        let key = (a.min(b), a.max(b));
        *self.at.entry(key).or_insert_with(|| {
            let x = mul(add(sites[a].map(|v| v as f64 * UNIT), sites[b].map(|v| v as f64 * UNIT)), 0.5);
            self.voxels.push(Voxel { x, m: 0.0, volume: f64::INFINITY, mat, held: held[a] || held[b] });
            self.around.push([sites[key.0], sites[key.1]]);
            self.voxels.len() - 1
        })
    }
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
    // Each fine part: the coarse cells it lies between.
    let between: Vec<[usize; 2]> = fine.around.iter().map(|a| [cell_of(a[0]), cell_of(a[1])]).collect();
    let n = sites.len();
    let mut cells: Vec<Cell> = (0..n).map(|_| Cell::default()).collect();
    let mut held = vec![false; n];
    let mut mass = vec![0.0; n];
    let mut mats: Vec<Vec<(Mat, f64)>> = vec![Vec::new(); n];
    let mut place: Vec<Option<(usize, Place)>> = vec![None; fine.voxels.len()];
    let mut on_face: HashMap<(usize, usize), f64> = HashMap::new();
    for (v, &[x, y]) in between.iter().enumerate() {
        let part = &fine.voxels[v];
        for c in [x, y] {
            held[c] |= part.held;
            match mats[c].iter_mut().find(|(m, _)| m.name == part.mat.name) {
                Some((_, w)) => *w += part.m,
                None => mats[c].push((part.mat, part.m)),
            }
        }
        if x == y {
            place[v] = Some((x, Place::Free(cells[x].free.len())));
            cells[x].free.push(v);
            mass[x] += part.m;
        } else {
            *on_face.entry((x.min(y), x.max(y))).or_insert(0.0) += part.m;
        }
    }
    // Each fine bond into its cell: whole where it lies inside one, else
    // half in each of the two it crosses between.
    for (b, bond) in fine.bonds.iter().enumerate() {
        let (ends_i, ends_j) = (between[bond.i], between[bond.j]);
        let inside = fine.around[bond.i].iter().find(|s| fine.around[bond.j].contains(s)).map(|s| index[&coarse_site(*s, level)]);
        let inside = inside.or((ends_i[0] == ends_i[1] && ends_i == ends_j).then_some(ends_i[0]));
        let at = |cells: &mut [Cell], c: usize, v: usize| match place[v] {
            Some((x, p)) if x == c => p,
            _ => cells[c].handle(if between[v][0] == c { between[v][1] } else { between[v][0] }),
        };
        match inside {
            Some(c) => {
                let (pi, pj) = (at(&mut cells, c, bond.i), at(&mut cells, c, bond.j));
                cells[c].bonds.push((pi, pj, b, 1.0));
            }
            None => {
                let (ci, cj) = (ends_i[0], ends_j[0]);
                let (pi, hj) = (at(&mut cells, ci, bond.i), cells[ci].handle(cj));
                cells[ci].bonds.push((pi, hj, b, 0.5));
                let (hi, pj) = (cells[cj].handle(ci), at(&mut cells, cj, bond.j));
                cells[cj].bonds.push((hi, pj, b, 0.5));
            }
        }
    }
    let mat_of = |c: usize| mats[c].iter().max_by(|a, b| a.1.total_cmp(&b.1)).map_or(STONE, |m| m.0);
    let mut nodes = Nodes { at: HashMap::new(), voxels: Vec::new(), around: Vec::new() };
    for (&(a, b), &m) in &on_face {
        let k = nodes.on(a, b, &sites, &held, mat_of(a));
        nodes.voxels[k].m += m;
    }
    let mut bonds = Vec::new();
    for (c, cell) in cells.iter().enumerate() {
        if cell.across.is_empty() {
            continue;
        }
        let mid = |o: usize| mul(add(sites[c].map(|v| v as f64 * UNIT), sites[o].map(|v| v as f64 * UNIT)), 0.5);
        let at: Vec<V3> = cell.across.iter().map(|&o| mid(o)).collect();
        let ends: Vec<usize> = cell.across.iter().map(|&o| nodes.on(c, o, &sites, &held, mat_of(c))).collect();
        let (found, used) = if held[c] {
            (Vec::new(), Vec::new())
        } else {
            let (k, used) = stiffness(fine, cell, &at);
            (beams(&k, &at), used)
        };
        // The cell's weight on its faces, as much as each is joined.
        let mut joined = vec![0.0; at.len()];
        for &(a, b, k) in &found {
            joined[a] += k;
            joined[b] += k;
        }
        let total: f64 = joined.iter().sum();
        for (h, &k) in ends.iter().enumerate() {
            nodes.voxels[k].m += mass[c] * if total > 0.0 { joined[h] / total } else { 1.0 / at.len() as f64 };
        }
        // How much of their strength the beams use, against the finer matter.
        let mat = mat_of(c);
        let weaker = if found.is_empty() { 1.0 } else { weakness(&found, &at, &used, mat) };
        for &(a, b, k) in &found {
            if !(nodes.voxels[ends[a]].held && nodes.voxels[ends[b]].held) {
                let length = norm(sub(at[b], at[a]));
                let m = Mat { ft: mat.ft / weaker, fc: mat.fc / weaker, fs: mat.fs / weaker, ..mat };
                bonds.extend(beam(ends[a], ends[b], at[a], at[b], k * length / mat.e, m));
            }
        }
    }
    let sites = nodes.around.iter().map(|a| a[0]).collect();
    Lattice { level, voxels: nodes.voxels, bonds, sites, around: nodes.around }
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
