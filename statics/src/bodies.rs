//! The forces in a structure under its weight, all voxels' at once, and
//! what of it stands.
//!
//! Every voxel is a rigid part, in equilibrium under its weight and its
//! bonds. Found by conjugate gradients, preconditioned by V-cycles over
//! bodies within bodies: what is left unbalanced goes up as resultants on
//! bodies, force and moment, and comes back as their rigid motions. So the
//! forces are those of all voxels, exactly; the bodies only find them
//! sooner, and no level holds a force it would have to answer for.

use crate::lattice::{Lattice, PER_FACE, coarse_site};
use crate::model::*;
use std::collections::HashMap;

type M6 = [[f64; 6]; 6];

/// Parts at one level, rigid, and the joints between them: each joint the
/// bonds between two parts, summed into one stiffness of their relative
/// motion at a point. A joint's side is a part, or None: held, it does
/// not move.
struct Level {
    centre: Vec<V3>,
    joints: Vec<Joint>,
    inverse: Vec<M6>,
    /// Each part's body at the next level up.
    up: Vec<usize>,
    /// At the top, the whole stiffness, factored.
    direct: Option<Cholesky>,
}

struct Joint {
    i: Option<usize>,
    j: Option<usize>,
    at: V3,
    stiffness: M6,
}

/// A part's motion (translation at its centre, rotation) as seen at a
/// point r from its centre: translation there, rotation.
pub fn shift(x: Six, r: V3) -> Six {
    let u = add([x[0], x[1], x[2]], cross([x[3], x[4], x[5]], r));
    [u[0], u[1], u[2], x[3], x[4], x[5]]
}

/// A force and moment at a point r from a part's centre, about its centre.
fn about(w: Six, r: V3) -> Six {
    let m = add([w[3], w[4], w[5]], cross(r, [w[0], w[1], w[2]]));
    [w[0], w[1], w[2], m[0], m[1], m[2]]
}

fn times(a: &M6, x: Six) -> Six {
    std::array::from_fn(|s| (0..6).map(|t| a[s][t] * x[t]).sum())
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

impl Level {
    fn new(lattice: &Lattice, part: &[Option<usize>], parts: usize, live: &[usize]) -> Level {
        let (voxels, bonds) = (&lattice.voxels, &lattice.bonds);
        let mut mass = vec![0.0; parts];
        let mut centre = vec![[0.0; 3]; parts];
        for (v, p) in part.iter().enumerate() {
            if let Some(k) = *p {
                mass[k] += voxels[v].m;
                centre[k] = add(centre[k], mul(voxels[v].x, voxels[v].m));
            }
        }
        for k in 0..parts {
            centre[k] = mul(centre[k], 1.0 / mass[k]);
        }
        let mut between: HashMap<(Option<usize>, Option<usize>), Vec<usize>> = HashMap::new();
        for &b in live {
            let (i, j) = (part[bonds[b].i], part[bonds[b].j]);
            if i != j {
                between.entry((i, j)).or_default().push(b);
            }
        }
        let joints: Vec<Joint> = between
            .into_iter()
            .map(|((i, j), list)| {
                let at = mul(list.iter().fold([0.0; 3], |s, &b| add(s, bonds[b].p)), 1.0 / list.len() as f64);
                // Each bond: the force at its point for the relative motion seen there.
                let stiffness = matrix(|d| {
                    list.iter().fold([0.0; 6], |w, &b| {
                        let r = sub(bonds[b].p, at);
                        let moved = shift(d, r);
                        let f = bonds[b].force([moved[0], moved[1], moved[2]]);
                        let here = about([f[0], f[1], f[2], 0.0, 0.0, 0.0], r);
                        std::array::from_fn(|s| w[s] + here[s])
                    })
                });
                Joint { i, j, at, stiffness }
            })
            .collect();
        let mut block = vec![[[0.0; 6]; 6]; parts];
        for joint in &joints {
            for k in [joint.i, joint.j].into_iter().flatten() {
                let r = sub(joint.at, centre[k]);
                let seen = matrix(|x| about(times(&joint.stiffness, shift(x, r)), r));
                for s in 0..6 {
                    for t in 0..6 {
                        block[k][s][t] += seen[s][t];
                    }
                }
            }
        }
        Level { centre, joints, inverse: block.iter().map(invert).collect(), up: Vec::new(), direct: None }
    }

    /// The loads that hold the parts moved by x.
    fn apply(&self, x: &[Six], work: &mut Work) -> Vec<Six> {
        work.0 += (x.len() + self.joints.len()) as f64;
        let mut y = vec![[0.0; 6]; x.len()];
        let seen = |k: Option<usize>, at: V3| k.map_or([0.0; 6], |k| shift(x[k], sub(at, self.centre[k])));
        for joint in &self.joints {
            let (a, b) = (seen(joint.i, joint.at), seen(joint.j, joint.at));
            let w = times(&joint.stiffness, std::array::from_fn(|s| b[s] - a[s]));
            for (side, sign) in [(joint.j, 1.0), (joint.i, -1.0)] {
                if let Some(k) = side {
                    let on = about(w, sub(joint.at, self.centre[k]));
                    for s in 0..6 {
                        y[k][s] += sign * on[s];
                    }
                }
            }
        }
        y
    }

    fn jacobi(&self, r: &[Six]) -> Vec<Six> {
        r.iter().zip(&self.inverse).map(|(v, m)| times(m, *v)).collect()
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

/// One V-cycle from level l: smooth, hand the rest up as resultants on
/// the bodies of the next level, take their rigid motions back, smooth.
fn cycle(levels: &[Level], l: usize, r: &[Six], work: &mut Work) -> Vec<Six> {
    let level = &levels[l];
    if l + 1 == levels.len() {
        return level.direct.as_ref().unwrap().solve(r);
    }
    let up = &levels[l + 1];
    let mut x = level.jacobi(r);
    let rest = |x: &[Six], work: &mut Work| -> Vec<Six> {
        let ax = level.apply(x, work);
        r.iter().zip(&ax).map(|(a, b)| std::array::from_fn(|s| a[s] - b[s])).collect()
    };
    let left = rest(&x, work);
    let mut coarse = vec![[0.0; 6]; up.centre.len()];
    for (k, v) in left.iter().enumerate() {
        let c = level.up[k];
        let w = about(*v, sub(level.centre[k], up.centre[c]));
        for s in 0..6 {
            coarse[c][s] += w[s];
        }
    }
    let y = cycle(levels, l + 1, &coarse, work);
    for k in 0..x.len() {
        let c = level.up[k];
        let u = shift(y[c], sub(level.centre[k], up.centre[c]));
        for s in 0..6 {
            x[k][s] += u[s];
        }
    }
    let left = rest(&x, work);
    for (xk, d) in x.iter_mut().zip(level.jacobi(&left)) {
        for s in 0..6 {
            xk[s] += d[s];
        }
    }
    x
}

/// Each bond's force, on j's side (on i's, the opposite), under weight:
/// `alive` bonds between voxels not `gone`, the rest none.
pub fn forces(lattice: &Lattice, alive: &[bool], gone: &[bool], work: &mut Work) -> Vec<V3> {
    let weight = |v: usize| [0.0, -lattice.voxels[v].m * GRAVITY, 0.0, 0.0, 0.0, 0.0];
    let x = motions(lattice, alive, gone, &weight, work);
    let mut f = moved(lattice, &x);
    for (b, bond) in lattice.bonds.iter().enumerate() {
        if !alive[b] || gone[bond.i] || gone[bond.j] {
            f[b] = [0.0; 3];
        }
    }
    f
}

/// Each bond's force, on j's side, its voxels moved by x.
pub fn moved(lattice: &Lattice, x: &[Six]) -> Vec<V3> {
    let at = |v: usize, p: V3| -> V3 { shift(x[v], sub(p, lattice.voxels[v].x))[..3].try_into().unwrap() };
    lattice.bonds.iter().map(|b| b.force(sub(at(b.j, b.p), at(b.i, b.p)))).collect()
}

/// Each voxel's motion (translation, rotation about its centre) under
/// `loads`, held ones and `gone` ones at rest: all voxels at once, by
/// conjugate gradients, preconditioned by V-cycles over bodies within
/// bodies, till few are left, and those solved directly.
pub fn motions(lattice: &Lattice, alive: &[bool], gone: &[bool], loads: &dyn Fn(usize) -> Six, work: &mut Work) -> Vec<Six> {
    const FEW: usize = 64;
    let (voxels, bonds) = (&lattice.voxels, &lattice.bonds);
    let live: Vec<usize> = (0..bonds.len()).filter(|&b| alive[b] && !gone[bonds[b].i] && !gone[bonds[b].j]).collect();
    let mut part: Vec<Option<usize>> = vec![None; voxels.len()];
    let mut count = 0;
    for v in 0..voxels.len() {
        if !voxels[v].held && !gone[v] {
            part[v] = Some(count);
            count += 1;
        }
    }
    let voxel = part.clone();
    let mut levels = vec![Level::new(lattice, &part, count, &live)];
    let mut key = lattice.sites.clone();
    let mut l = lattice.level;
    while count > FEW {
        l += 1;
        for k in key.iter_mut() {
            *k = coarse_site(*k, l);
        }
        let (up, next, bodies) = coarser(levels.last().unwrap(), &part, &key);
        if bodies == count {
            break;
        }
        levels.last_mut().unwrap().up = up;
        (part, count) = (next, bodies);
        levels.push(Level::new(lattice, &part, count, &live));
    }
    let top = levels.last_mut().unwrap();
    top.direct = Some(Cholesky::of(top));
    let loads: Vec<Six> = (0..voxels.len()).filter(|&v| voxel[v].is_some()).map(loads).collect();
    let mut inner = Work(0.0);
    let mut v_cycle = |r: &[Six]| cycle(&levels, 0, r, &mut inner);
    let x = conjugate_gradients(&levels[0], &loads, &mut v_cycle, 1e-12, work);
    work.0 += inner.0;
    (0..voxels.len()).map(|v| voxel[v].map_or([0.0; 6], |k| x[k])).collect()
}

/// The bodies of the next level: the connected pieces of each key's
/// parts. Each part's body, each voxel's, and how many.
fn coarser(level: &Level, part: &[Option<usize>], key: &[[i64; 3]]) -> (Vec<usize>, Vec<Option<usize>>, usize) {
    let parts = level.centre.len();
    let mut their = vec![[0; 3]; parts];
    for (v, p) in part.iter().enumerate() {
        if let Some(k) = *p {
            their[k] = key[v];
        }
    }
    let mut parent: Vec<usize> = (0..parts).collect();
    for joint in &level.joints {
        if let (Some(i), Some(j)) = (joint.i, joint.j)
            && their[i] == their[j]
        {
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

/// Find the forces; break every face with a bond whose stress is over its
/// strength; let fall what is no longer joined to anything held; again,
/// until nothing breaks.
pub fn settle(lattice: &Lattice) -> Outcome {
    let (voxels, bonds) = (&lattice.voxels, &lattice.bonds);
    let mut alive = vec![true; bonds.len()];
    let mut gone = vec![false; voxels.len()];
    let mut work = Work(0.0);
    let mut rounds = 0;
    loop {
        rounds += 1;
        let held = joined_to_held(voxels, bonds, &alive);
        for v in 0..voxels.len() {
            gone[v] |= !held[v];
        }
        let f = forces(lattice, &alive, &gone, &mut work);
        let u = used(lattice, &f);
        let over: Vec<bool> = bonds.iter().enumerate().map(|(b, bond)| alive[b] && !gone[bond.i] && !gone[bond.j] && u[b] > 1.0).collect();
        // A face breaks whole: what is joined at all is joined rigidly.
        let mut broke = false;
        for (face, bonds) in over.chunks(PER_FACE).enumerate() {
            if bonds.contains(&true) {
                alive[face * PER_FACE..][..PER_FACE].fill(false);
                broke = true;
            }
        }
        if !broke {
            break;
        }
    }
    let free: f64 = voxels.iter().filter(|v| !v.held).map(|v| v.m).sum();
    let fell: f64 = voxels.iter().zip(&gone).filter(|(_, g)| **g).map(|(v, _)| v.m).sum();
    Outcome { fallen: if free > 0.0 { fell / free } else { 0.0 }, rounds, work: work.0 }
}

/// The share of its strength each bond uses, bearing `force`: at a bond,
/// its free voxels' stress, on average, but on its face what it bears.
pub fn used(lattice: &Lattice, force: &[V3]) -> Vec<f64> {
    let s = stresses(lattice, force);
    let voxels = &lattice.voxels;
    let bonds = lattice.bonds.iter().zip(force);
    bonds
        .map(|(b, f)| {
            let ends: Vec<usize> = [b.i, b.j].into_iter().filter(|&v| !voxels[v].held).collect();
            let mean: M3 =
                std::array::from_fn(|r| std::array::from_fn(|c| ends.iter().map(|&v| s[v][r][c]).sum::<f64>() / ends.len().max(1) as f64));
            b.mat.utilisation(&b.stress(&mean, *f))
        })
        .collect()
}

/// What `used` is with everything standing as it is.
pub fn used_standing(lattice: &Lattice) -> Vec<f64> {
    let alive = vec![true; lattice.bonds.len()];
    let gone = vec![false; lattice.voxels.len()];
    used(lattice, &forces(lattice, &alive, &gone, &mut Work(0.0)))
}

/// Each voxel's stress: the forces on it from its bonds, each times where
/// it acts from the voxel's centre, over its volume.
fn stresses(lattice: &Lattice, force: &[V3]) -> Vec<M3> {
    let mut s = vec![[[0.0; 3]; 3]; lattice.voxels.len()];
    for (bond, f) in lattice.bonds.iter().zip(force) {
        for (v, on) in [(bond.i, *f), (bond.j, mul(*f, -1.0))] {
            let r = sub(bond.p, lattice.voxels[v].x);
            for a in 0..3 {
                for c in 0..3 {
                    s[v][a][c] += (r[a] * on[c] + r[c] * on[a]) / (2.0 * lattice.voxels[v].volume);
                }
            }
        }
    }
    s
}

/// Which voxels a chain of bonds joins to a held one.
pub fn joined_to_held(voxels: &[Voxel], bonds: &[Bond], alive: &[bool]) -> Vec<bool> {
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); voxels.len()];
    for (bond, _) in bonds.iter().zip(alive).filter(|(_, a)| **a) {
        next[bond.i].push(bond.j);
        next[bond.j].push(bond.i);
    }
    let mut held: Vec<bool> = voxels.iter().map(|v| v.held).collect();
    let mut stack: Vec<usize> = (0..voxels.len()).filter(|&v| held[v]).collect();
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
/// each cell of `generated` loaded under its weight, the most of its
/// strength any bond in it uses; `lattice`'s bonds in the same cells, so
/// much stronger at least.
pub fn as_generated(lattice: &mut Lattice, generated: &Lattice) {
    let mut need: HashMap<[i64; 3], f64> = HashMap::new();
    for (b, u) in generated.bonds.iter().zip(used_standing(generated)) {
        for cell in generated.cells(b) {
            let n = need.entry(cell).or_insert(0.0);
            *n = n.max(u);
        }
    }
    let stronger = |b: &Bond| lattice.cells(b).iter().map(|c| need.get(c).map_or(0.0, |u| u * MARGIN)).fold(1.0, f64::max);
    let by: Vec<f64> = lattice.bonds.iter().map(stronger).collect();
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
