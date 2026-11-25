//! The forces in a structure under its weight, all nodes' at once, and
//! what of it stands.
//!
//! Every node is a rigid part, in equilibrium under its weight and its
//! bonds. Found by conjugate gradients, preconditioned by V-cycles over
//! bodies within bodies (smoothed aggregation): what is left unbalanced
//! goes up to bodies, each moving as a rigid body smoothed by a step of
//! what its neighbours ask, and comes back as their motions. So the
//! forces are those of all nodes, exactly; the bodies only find them
//! sooner, and no level holds a force it would have to answer for.

use crate::lattice::{Lattice, PER_BEAM, coarse_site};
use crate::model::*;
use std::collections::HashMap;

type M6 = [[f64; 6]; 6];

/// A level of the solver: its parts' stiffness, block by block (each
/// part's own, and each pair's, i < j: what j's motion asks of i; the
/// other way, its transpose), and how its parts move with the next
/// level's. Level 0's parts are the nodes, its stiffness their joints';
/// a coarser level's parts are bodies of the finer one's, each moving as
/// a rigid body would, smoothed by a step of what its neighbours ask, its
/// stiffness what the finer one's is under those motions (Galerkin).
struct Level {
    centre: Vec<V3>,
    own: Vec<M6>,
    pairs: Vec<(usize, usize, M6)>,
    /// Each part's pairs: the other part, the pair, whether transposed.
    rows: Vec<Vec<(usize, usize, bool)>>,
    inverse: Vec<M6>,
    /// How far a step of the smoother may go: 4/3 over the most it asks.
    step: f64,
    /// Each part's motion as the next level's parts', block by block.
    down: Vec<Vec<(usize, M6)>>,
    /// Each of the next level's parts: where it is in `down`, part by part.
    from: Vec<Vec<(usize, usize)>>,
    /// At the top, the whole stiffness, factored.
    direct: Option<Cholesky>,
}

/// A part's motion (translation at its centre, rotation) as seen at a
/// point r from its centre: translation there, rotation.
pub fn shift(x: Six, r: V3) -> Six {
    let u = add([x[0], x[1], x[2]], cross([x[3], x[4], x[5]], r));
    [u[0], u[1], u[2], x[3], x[4], x[5]]
}

/// f of each of 0..n, on as many threads as there are cores, unless n is
/// few.
pub fn each<T: Send + Default + Clone>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get());
    if n < 256 || threads == 1 {
        return (0..n).map(f).collect();
    }
    let mut out = vec![T::default(); n];
    let size = n.div_ceil(threads);
    std::thread::scope(|scope| {
        for (c, chunk) in out.chunks_mut(size).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (k, o) in chunk.iter_mut().enumerate() {
                    *o = f(c * size + k);
                }
            });
        }
    });
    out
}

fn times(a: &M6, x: Six) -> Six {
    std::array::from_fn(|s| (0..6).map(|t| a[s][t] * x[t]).sum())
}

fn times_transposed(a: &M6, x: Six) -> Six {
    std::array::from_fn(|s| (0..6).map(|t| a[t][s] * x[t]).sum())
}

fn add6v(a: Six, b: Six) -> Six {
    std::array::from_fn(|s| a[s] + b[s])
}

fn product(a: &M6, b: &M6) -> M6 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..6).map(|k| a[r][k] * b[k][c]).sum()))
}

fn transpose(a: &M6) -> M6 {
    std::array::from_fn(|r| std::array::from_fn(|c| a[c][r]))
}

fn add6(to: &mut M6, a: &M6, by: f64) {
    for r in 0..6 {
        for c in 0..6 {
            to[r][c] += by * a[r][c];
        }
    }
}

/// The matrix of a linear map of sixes, column by column.
fn matrix(f: impl Fn(Six) -> Six) -> M6 {
    let mut a = [[0.0; 6]; 6];
    for c in 0..6 {
        let col = f(std::array::from_fn(|s| if s == c { 1.0 } else { 0.0 }));
        for r in 0..6 {
            a[r][c] = col[r];
        }
    }
    a
}

/// Add `by`·m to row's block at c.
fn merge(row: &mut Vec<(usize, M6)>, c: usize, m: &M6, by: f64) {
    match row.iter_mut().find(|(d, _)| *d == c) {
        Some((_, to)) => add6(to, m, by),
        None => {
            let mut to = [[0.0; 6]; 6];
            add6(&mut to, m, by);
            row.push((c, to));
        }
    }
}

impl Level {
    /// Level 0: the nodes not held, and the stiffness of their `live`
    /// bonds; `part` is each node's index here, if any.
    fn new(lattice: &Lattice, part: &[Option<usize>], parts: usize, live: &[usize]) -> Level {
        let (nodes, bonds) = (&lattice.nodes, &lattice.bonds);
        let mut centre = vec![[0.0; 3]; parts];
        for (v, p) in part.iter().enumerate() {
            if let Some(k) = *p {
                centre[k] = nodes[v].x;
            }
        }
        let mut own = vec![[[0.0; 6]; 6]; parts];
        let mut pairs: Vec<(usize, usize, M6)> = Vec::new();
        let mut pair: HashMap<(usize, usize), usize> = HashMap::new();
        for &b in live {
            let bond = &bonds[b];
            let s = bond.stiffness();
            // How a node's motion moves the bond's point: T = [1 | −r×].
            let seen = |v: usize| -> [[f64; 6]; 3] {
                let r = sub(bond.p, nodes[v].x);
                [[1.0, 0.0, 0.0, 0.0, r[2], -r[1]], [0.0, 1.0, 0.0, -r[2], 0.0, r[0]], [0.0, 0.0, 1.0, r[1], -r[0], 0.0]]
            };
            // Tᵢᵀ·S·Tⱼ: what j's motion asks of i through the bond.
            let through = |a: &[[f64; 6]; 3], b: &[[f64; 6]; 3]| -> M6 {
                let sb: [[f64; 6]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| s[r][k] * b[k][c]).sum()));
                std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[k][r] * sb[k][c]).sum()))
            };
            let side = [bond.i, bond.j].map(|v| part[v].map(|k| (k, seen(v))));
            for (k, t) in side.iter().flatten() {
                add6(&mut own[*k], &through(t, t), 1.0);
            }
            if let [Some((i, a)), Some((j, b))] = side {
                let (i, j, a, b) = if i < j { (i, j, a, b) } else { (j, i, b, a) };
                let k = *pair.entry((i, j)).or_insert_with(|| {
                    pairs.push((i, j, [[0.0; 6]; 6]));
                    pairs.len() - 1
                });
                add6(&mut pairs[k].2, &through(&a, &b), -1.0);
            }
        }
        Level::of(centre, own, pairs)
    }

    fn of(centre: Vec<V3>, own: Vec<M6>, pairs: Vec<(usize, usize, M6)>) -> Level {
        let mut rows = vec![Vec::new(); own.len()];
        for (p, &(i, j, _)) in pairs.iter().enumerate() {
            rows[i].push((j, p, false));
            rows[j].push((i, p, true));
        }
        let inverse = own.iter().map(invert).collect();
        let mut level = Level { centre, own, pairs, rows, inverse, step: 1.0, down: Vec::new(), from: Vec::new(), direct: None };
        level.step = 4.0 / 3.0 / level.most_asked();
        level
    }

    /// The greatest eigenvalue of the stiffness over each part's own, by
    /// a few powers of it.
    fn most_asked(&self) -> f64 {
        let mut v: Vec<Six> = (0..self.own.len()).map(|k| std::array::from_fn(|s| 1.0 + ((7 * k + 3 * s) % 11) as f64 / 11.0)).collect();
        let mut most = 1.0;
        for _ in 0..12 {
            let w = self.jacobi(&self.apply(&v, &mut Work(0.0)));
            most = dot6(&w, &w).sqrt() / dot6(&v, &v).sqrt();
            let scale = 1.0 / dot6(&w, &w).sqrt();
            v = w.iter().map(|x| x.map(|a| a * scale)).collect();
        }
        most
    }

    /// The loads that hold the parts moved by x.
    fn apply(&self, x: &[Six], work: &mut Work) -> Vec<Six> {
        work.0 += (x.len() + self.pairs.len()) as f64;
        each(x.len(), |i| {
            let mut y = times(&self.own[i], x[i]);
            for &(o, p, transposed) in &self.rows[i] {
                let m = &self.pairs[p].2;
                let a = if transposed { times_transposed(m, x[o]) } else { times(m, x[o]) };
                for s in 0..6 {
                    y[s] += a[s];
                }
            }
            y
        })
    }

    fn jacobi(&self, r: &[Six]) -> Vec<Six> {
        each(r.len(), |i| times(&self.inverse[i], r[i]))
    }
    /// Row k of K·M, M's rows given part by part: what the motions M
    /// gives k and its neighbours ask of k.
    fn ask(&self, k: usize, m: &dyn Fn(usize) -> Vec<(usize, M6)>) -> Vec<(usize, M6)> {
        let mut row: Vec<(usize, M6)> = Vec::new();
        for (c, b) in m(k) {
            merge(&mut row, c, &product(&self.own[k], &b), 1.0);
        }
        for &(o, p, transposed) in &self.rows[k] {
            let a = if transposed { transpose(&self.pairs[p].2) } else { self.pairs[p].2 };
            for (c, b) in m(o) {
                merge(&mut row, c, &product(&a, &b), 1.0);
            }
        }
        row
    }

    /// The next level up, of bodies: each part's body (`up`), `count` of them.
    fn coarser(&mut self, up: &[usize], count: usize) -> Level {
        let mut centre = vec![[0.0; 3]; count];
        let mut n = vec![0.0; count];
        for (k, &c) in up.iter().enumerate() {
            centre[c] = add(centre[c], self.centre[k]);
            n[c] += 1.0;
        }
        for c in 0..count {
            centre[c] = mul(centre[c], 1.0 / n[c]);
        }
        // Rigid with its body, then a smoothing step: P = (1 − step·D⁻¹K)·P₀.
        let rigid: Vec<M6> = (0..up.len()).map(|k| matrix(|y| shift(y, sub(self.centre[k], centre[up[k]])))).collect();
        let rigid_row = |k: usize| vec![(up[k], rigid[k])];
        let down = each(up.len(), |k| {
            let mut row = rigid_row(k);
            for (c, m) in self.ask(k, &rigid_row) {
                merge(&mut row, c, &product(&self.inverse[k], &m), -self.step);
            }
            row
        });
        self.down = down;
        self.from = vec![Vec::new(); count];
        for (k, row) in self.down.iter().enumerate() {
            for (q, (c, _)) in row.iter().enumerate() {
                self.from[*c].push((k, q));
            }
        }
        // What the finer stiffness asks under those motions: K·P, then Pᵀ·K·P.
        let fine = &*self;
        let kp = each(up.len(), |k| fine.ask(k, &|o| fine.down[o].clone()));
        let rows = each(count, |c| {
            let mut row: Vec<(usize, M6)> = Vec::new();
            for &(k, q) in &fine.from[c] {
                let pt = transpose(&fine.down[k][q].1);
                for (d, m) in &kp[k] {
                    if *d >= c {
                        merge(&mut row, *d, &product(&pt, m), 1.0);
                    }
                }
            }
            row
        });
        let mut own = vec![[[0.0; 6]; 6]; count];
        let mut pairs: Vec<(usize, usize, M6)> = Vec::new();
        for (c, row) in rows.into_iter().enumerate() {
            for (d, m) in row {
                if d == c {
                    own[c] = m;
                } else {
                    pairs.push((c, d, m));
                }
            }
        }
        Level::of(centre, own, pairs)
    }
}

/// A symmetric positive matrix as L·Lᵀ, to solve with.
struct Cholesky {
    n: usize,
    l: Vec<f64>,
}

impl Cholesky {
    /// The stiffness of the level's parts, column by column.
    fn of(level: &Level) -> Cholesky {
        let parts = level.centre.len();
        let n = 6 * parts;
        let mut a = vec![0.0; n * n];
        let mut w = Work(0.0);
        for c in 0..n {
            let mut e = vec![[0.0; 6]; parts];
            e[c / 6][c % 6] = 1.0;
            for (k, v) in level.apply(&e, &mut w).iter().enumerate() {
                for s in 0..6 {
                    a[(6 * k + s) * n + c] = v[s];
                }
            }
        }
        for j in 0..n {
            let d = (a[j * n + j] - (0..j).map(|k| a[j * n + k] * a[j * n + k]).sum::<f64>()).sqrt();
            a[j * n + j] = d;
            for i in j + 1..n {
                a[i * n + j] = (a[i * n + j] - (0..j).map(|k| a[i * n + k] * a[j * n + k]).sum::<f64>()) / d;
            }
        }
        Cholesky { n, l: a }
    }

    fn solve(&self, r: &[Six]) -> Vec<Six> {
        let n = self.n;
        let mut y: Vec<f64> = r.iter().flat_map(|v| v.iter().copied()).collect();
        for i in 0..n {
            y[i] = (y[i] - (0..i).map(|k| self.l[i * n + k] * y[k]).sum::<f64>()) / self.l[i * n + i];
        }
        for i in (0..n).rev() {
            y[i] = (y[i] - (i + 1..n).map(|k| self.l[k * n + i] * y[k]).sum::<f64>()) / self.l[i * n + i];
        }
        y.chunks(6).map(|c| std::array::from_fn(|s| c[s])).collect()
    }
}

/// What a solve took: iterations, each times the parts and bonds in it.
pub struct Work(pub f64);

fn dot6(a: &[Six], b: &[Six]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (0..6).map(|s| x[s] * y[s]).sum::<f64>()).sum()
}

/// Conjugate gradients on a level, preconditioned by `precondition`.
fn conjugate_gradients(
    level: &Level,
    loads: &[Six],
    precondition: &mut dyn FnMut(&[Six]) -> Vec<Six>,
    tol: f64,
    work: &mut Work,
) -> Vec<Six> {
    let mut x = vec![[0.0; 6]; loads.len()];
    let mut r = loads.to_vec();
    let mut z = precondition(&r);
    let mut p = z.clone();
    let mut rz = dot6(&r, &z);
    let goal = tol * dot6(loads, loads).sqrt();
    for _ in 0..100_000 {
        if dot6(&r, &r).sqrt() <= goal {
            break;
        }
        let ap = level.apply(&p, work);
        let alpha = rz / dot6(&p, &ap);
        for k in 0..x.len() {
            for s in 0..6 {
                x[k][s] += alpha * p[k][s];
                r[k][s] -= alpha * ap[k][s];
            }
        }
        z = precondition(&r);
        let next = dot6(&r, &z);
        for k in 0..x.len() {
            for s in 0..6 {
                p[k][s] = z[k][s] + next / rz * p[k][s];
            }
        }
        rz = next;
    }
    x
}

/// One V-cycle from level l: smooth, hand the rest up to the next level,
/// take its motions back, smooth.
fn cycle(levels: &[Level], l: usize, r: &[Six], work: &mut Work) -> Vec<Six> {
    let level = &levels[l];
    if l + 1 == levels.len() {
        return level.direct.as_ref().unwrap().solve(r);
    }
    let rest = |x: &[Six], work: &mut Work| -> Vec<Six> {
        let ax = level.apply(x, work);
        r.iter().zip(&ax).map(|(a, b)| std::array::from_fn(|s| a[s] - b[s])).collect()
    };
    let mut x: Vec<Six> = level.jacobi(r).iter().map(|v| v.map(|a| a * level.step)).collect();
    let left = rest(&x, work);
    let coarse = each(level.from.len(), |c| {
        level.from[c].iter().fold([0.0; 6], |a, &(k, q)| add6v(a, times_transposed(&level.down[k][q].1, left[k])))
    });
    let y = cycle(levels, l + 1, &coarse, work);
    let moved = each(x.len(), |k| level.down[k].iter().fold([0.0; 6], |a, (c, p)| add6v(a, times(p, y[*c]))));
    for (xk, u) in x.iter_mut().zip(moved) {
        *xk = add6v(*xk, u);
    }
    let left = rest(&x, work);
    for (xk, d) in x.iter_mut().zip(level.jacobi(&left)) {
        for s in 0..6 {
            xk[s] += level.step * d[s];
        }
    }
    x
}

/// Each bond's force, on j's side (on i's, the opposite), under weight:
/// `alive` bonds between nodes not `gone`, the rest none.
pub fn forces(lattice: &Lattice, alive: &[bool], gone: &[bool], work: &mut Work) -> Vec<V3> {
    let weight = |v: usize| [0.0, -lattice.nodes[v].m * GRAVITY, 0.0, 0.0, 0.0, 0.0];
    let x = motions(lattice, alive, gone, &weight, work);
    let mut f = moved(lattice, &x);
    for (b, bond) in lattice.bonds.iter().enumerate() {
        if !alive[b] || gone[bond.i] || gone[bond.j] {
            f[b] = [0.0; 3];
        }
    }
    f
}

/// Each bond's force, on j's side, its nodes moved by x.
pub fn moved(lattice: &Lattice, x: &[Six]) -> Vec<V3> {
    let at = |v: usize, p: V3| -> V3 { shift(x[v], sub(p, lattice.nodes[v].x))[..3].try_into().unwrap() };
    lattice.bonds.iter().map(|b| b.force(sub(at(b.j, b.p), at(b.i, b.p)))).collect()
}

/// Each node's motion (translation, rotation about its centre) under
/// `loads`, held ones and `gone` ones at rest: all nodes at once, by
/// conjugate gradients, preconditioned by V-cycles over bodies within
/// bodies, till few are left, and those solved directly.
pub fn motions(lattice: &Lattice, alive: &[bool], gone: &[bool], loads: &dyn Fn(usize) -> Six, work: &mut Work) -> Vec<Six> {
    const FEW: usize = 64;
    // Verdicts at 1e-12 were the same, at half again the iterations.
    const TOLERANCE: f64 = 1e-6;
    let (nodes, bonds) = (&lattice.nodes, &lattice.bonds);
    let live: Vec<usize> = (0..bonds.len()).filter(|&b| alive[b] && !gone[bonds[b].i] && !gone[bonds[b].j]).collect();
    let mut part: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut count = 0;
    for v in 0..nodes.len() {
        if !nodes[v].held && !gone[v] {
            part[v] = Some(count);
            count += 1;
        }
    }
    let node = part.clone();
    let mut levels = vec![Level::new(lattice, &part, count, &live)];
    let mut key: Vec<[i64; 3]> = lattice.around.iter().map(|a| a[0]).collect();
    let mut l = lattice.level;
    while count > FEW {
        l += 1;
        for k in key.iter_mut() {
            *k = coarse_site(*k, l);
        }
        let (up, next, bodies) = bodies_of(levels.last().unwrap(), &part, &key);
        if bodies == count {
            break;
        }
        let up_level = levels.last_mut().unwrap().coarser(&up, bodies);
        levels.push(up_level);
        (part, count) = (next, bodies);
    }
    let top = levels.last_mut().unwrap();
    top.direct = Some(Cholesky::of(top));
    let loads: Vec<Six> = (0..nodes.len()).filter(|&v| node[v].is_some()).map(loads).collect();
    let mut inner = Work(0.0);
    let mut v_cycle = |r: &[Six]| cycle(&levels, 0, r, &mut inner);
    let x = conjugate_gradients(&levels[0], &loads, &mut v_cycle, TOLERANCE, work);
    work.0 += inner.0;
    (0..nodes.len()).map(|v| node[v].map_or([0.0; 6], |k| x[k])).collect()
}

/// The bodies of the next level: the connected pieces of each key's
/// parts. Each part's body, each node's, and how many.
fn bodies_of(level: &Level, part: &[Option<usize>], key: &[[i64; 3]]) -> (Vec<usize>, Vec<Option<usize>>, usize) {
    let parts = level.centre.len();
    let mut their = vec![[0; 3]; parts];
    for (v, p) in part.iter().enumerate() {
        if let Some(k) = *p {
            their[k] = key[v];
        }
    }
    let mut parent: Vec<usize> = (0..parts).collect();
    for &(i, j, _) in &level.pairs {
        if their[i] == their[j] {
            let (a, c) = (find(&mut parent, i), find(&mut parent, j));
            parent[a] = c;
        }
    }
    let mut id = HashMap::new();
    let up: Vec<usize> = (0..parts)
        .map(|k| {
            let n = id.len();
            *id.entry(find(&mut parent, k)).or_insert(n)
        })
        .collect();
    let next = part.iter().map(|p| p.map(|k| up[k])).collect();
    (up, next, id.len())
}

pub struct Outcome {
    /// The share of the free mass that fell.
    pub fallen: f64,
    /// Solves, and their work.
    pub rounds: usize,
    pub work: f64,
}

/// Find the forces; break every beam with a bond whose stress is over its
/// strength; let fall what is no longer joined to anything held; again,
/// until nothing breaks.
pub fn settle(lattice: &Lattice) -> Outcome {
    let (nodes, bonds) = (&lattice.nodes, &lattice.bonds);
    let mut alive = vec![true; bonds.len()];
    let mut gone = vec![false; nodes.len()];
    let mut work = Work(0.0);
    let mut rounds = 0;
    loop {
        rounds += 1;
        let held = joined_to_held(nodes, bonds, &alive);
        for v in 0..nodes.len() {
            gone[v] |= !held[v];
        }
        let f = forces(lattice, &alive, &gone, &mut work);
        let u = used(lattice, &f);
        let over: Vec<bool> = bonds.iter().enumerate().map(|(b, bond)| alive[b] && !gone[bond.i] && !gone[bond.j] && u[b] > 1.0).collect();
        // A beam breaks whole: what is joined at all is joined rigidly.
        let mut broke = false;
        for (beam, bonds) in over.chunks(PER_BEAM).enumerate() {
            if bonds.contains(&true) {
                alive[beam * PER_BEAM..][..PER_BEAM].fill(false);
                broke = true;
            }
        }
        if !broke {
            break;
        }
    }
    let free: f64 = nodes.iter().filter(|v| !v.held).map(|v| v.m).sum();
    let fell: f64 = nodes.iter().zip(&gone).filter(|(_, g)| **g).map(|(v, _)| v.m).sum();
    Outcome { fallen: if free > 0.0 { fell / free } else { 0.0 }, rounds, work: work.0 }
}

/// The share of its strength each bond uses, bearing `force`.
pub fn used(lattice: &Lattice, force: &[V3]) -> Vec<f64> {
    lattice.bonds.iter().zip(force).map(|(b, f)| b.used(*f)).collect()
}

/// What `used` is with everything standing as it is: all that holds to
/// the ground, nothing broken.
pub fn used_standing(lattice: &Lattice) -> Vec<f64> {
    let alive = vec![true; lattice.bonds.len()];
    let gone: Vec<bool> = joined_to_held(&lattice.nodes, &lattice.bonds, &alive).iter().map(|h| !h).collect();
    used(lattice, &forces(lattice, &alive, &gone, &mut Work(0.0)))
}

/// Which nodes a chain of bonds joins to a held one.
pub fn joined_to_held(nodes: &[Node], bonds: &[Bond], alive: &[bool]) -> Vec<bool> {
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (bond, _) in bonds.iter().zip(alive).filter(|(_, a)| **a) {
        next[bond.i].push(bond.j);
        next[bond.j].push(bond.i);
    }
    let mut held: Vec<bool> = nodes.iter().map(|v| v.held).collect();
    let mut stack: Vec<usize> = (0..nodes.len()).filter(|&v| held[v]).collect();
    while let Some(v) = stack.pop() {
        for &w in &next[v] {
            if !held[w] {
                held[w] = true;
                stack.push(w);
            }
        }
    }
    held
}

fn invert(m: &M6) -> M6 {
    let mut a = *m;
    let mut inv = [[0.0; 6]; 6];
    for k in 0..6 {
        inv[k][k] = 1.0;
    }
    for col in 0..6 {
        let piv = (col..6).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs())).unwrap();
        a.swap(col, piv);
        inv.swap(col, piv);
        let d = a[col][col];
        for k in 0..6 {
            a[col][k] /= d;
            inv[col][k] /= d;
        }
        for r in 0..6 {
            let f = a[r][col];
            if r != col && f != 0.0 {
                for k in 0..6 {
                    a[r][k] -= f * a[col][k];
                    inv[r][k] -= f * inv[col][k];
                }
            }
        }
    }
    inv
}

/// How much stronger than its material a world as generated stands.
pub const MARGIN: f64 = 2.0;

/// Strength no less than what a world as generated needs, MARGIN times:
/// each beam of `generated` loaded under its weight, the most of its
/// strength any of its bonds uses; the same beam of `lattice` (between
/// the same faces), so much stronger at least.
pub fn as_generated(lattice: &mut Lattice, generated: &Lattice) {
    let beam = |l: &Lattice, b: &Bond| {
        let (i, j) = (l.around[b.i], l.around[b.j]);
        if i < j { [i, j] } else { [j, i] }
    };
    let mut need: HashMap<[[[i64; 3]; 2]; 2], f64> = HashMap::new();
    for (b, u) in generated.bonds.iter().zip(used_standing(generated)) {
        let n = need.entry(beam(generated, b)).or_insert(0.0);
        *n = n.max(u);
    }
    let by: Vec<f64> = lattice.bonds.iter().map(|b| need.get(&beam(lattice, b)).map_or(1.0, |u| (u * MARGIN).max(1.0))).collect();
    for (b, by) in lattice.bonds.iter_mut().zip(by) {
        b.mat = b.mat.stronger(by);
    }
}

/// The root of a's set, a forest of parents halved as it is walked.
pub(crate) fn find(p: &mut [usize], mut a: usize) -> usize {
    while p[a] != a {
        p[a] = p[p[a]];
        a = p[a];
    }
    a
}
