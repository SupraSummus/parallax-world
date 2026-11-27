//! A symmetric system over cells, three unknowns to a cell and a block to
//! each pair joined, solved by conjugate gradients preconditioned by
//! V-cycles over the octree.
//!
//! A level up, a part is the cells of one cell of the octree that are
//! joined (an aggregate), and it moves as a rigid body: six unknowns,
//! its translation at its middle and its turn. Its stiffness is what its
//! cells' is under those motions (Galerkin), and so on up, till a few
//! parts are solved directly. What a level leaves unbalanced goes up as
//! forces and torques, and comes back as motions. So what moves together
//! at large, a tree falling, is found at the level it is large at, and
//! the cells only bend about it: the levels of detail the worlds are
//! drawn at are the solver's.

pub type V3 = [f64; 3];
pub type M3 = [[f64; 3]; 3];
type V6 = [f64; 6];
type M6 = [[f64; 6]; 6];

/** Of the solve: how many rounds at most. */
const ROUNDS: usize = 300;
/** Parts few enough to solve directly. */
const DIRECT: usize = 24;

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    a.map(|v| v * s)
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn times(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
pub fn times_t(m: &M3, v: V3) -> V3 {
    std::array::from_fn(|c| (0..3).map(|r| m[r][c] * v[r]).sum())
}

/**
 * The system: each cell's site (for the octree) and where it is (for
 * rigid motions), its block on the diagonal, and each pair's (i, j, B):
 * the block in row i, column j, its transpose in row j, column i; a pair
 * twice is the sum of its blocks.
 */
#[derive(Clone, Default)]
pub struct System {
    pub sites: Vec<[i64; 3]>,
    pub at: Vec<V3>,
    pub diag: Vec<M3>,
    pub pairs: Vec<(u32, u32, M3)>,
}

impl System {
    pub fn times(&self, x: &[V3], y: &mut [V3]) {
        for (i, d) in self.diag.iter().enumerate() {
            y[i] = times(d, x[i]);
        }
        for &(i, j, ref b) in &self.pairs {
            let (i, j) = (i as usize, j as usize);
            y[i] = add(y[i], times(b, x[j]));
            y[j] = add(y[j], times_t(b, x[i]));
        }
    }
}

/** T(r)ᵀ M T(s), where T(r) moves a rigid motion from a part's middle to r from it: what M is between parts at r and s. */
fn lift(m: &M6, r: V3, s: V3) -> M6 {
    // T(r) = [[I, X(r)], [0, I]], X(r) w = w × r.
    let x = |r: V3| -> M3 { [[0.0, r[2], -r[1]], [-r[2], 0.0, r[0]], [r[1], -r[0], 0.0]] };
    let (xr, xs) = (x(r), x(s));
    let mut t = *m;
    // Columns: M T(s): the right half gains M's left half times X(s).
    for row in 0..6 {
        for c in 0..3 {
            t[row][3 + c] += (0..3).map(|k| m[row][k] * xs[k][c]).sum::<f64>();
        }
    }
    // Rows: T(r)ᵀ (M T(s)): the lower half gains X(r)ᵀ times the upper half.
    let mut out = t;
    for r3 in 0..3 {
        for col in 0..6 {
            out[3 + r3][col] += (0..3).map(|k| xr[k][r3] * t[k][col]).sum::<f64>();
        }
    }
    out
}

fn embed(b: &M3) -> M6 {
    let mut m = [[0.0; 6]; 6];
    for r in 0..3 {
        m[r][..3].copy_from_slice(&b[r]);
    }
    m
}

fn transpose6(m: &M6) -> M6 {
    std::array::from_fn(|r| std::array::from_fn(|c| m[c][r]))
}

fn plus6(a: &mut M6, b: &M6) {
    for r in 0..6 {
        for c in 0..6 {
            a[r][c] += b[r][c];
        }
    }
}

fn times6(m: &M6, v: &V6) -> V6 {
    std::array::from_fn(|r| (0..6).map(|c| m[r][c] * v[c]).sum())
}

fn times6t(m: &M6, v: &V6) -> V6 {
    std::array::from_fn(|r| (0..6).map(|c| m[c][r] * v[c]).sum())
}

/** The inverse of a symmetric matrix that is positive or nearly so, by Cholesky; a dof it does not reach stays still. */
fn inverse<const N: usize>(m: &[[f64; N]; N]) -> [[f64; N]; N] {
    let n = N;
    let mut a: Vec<f64> = m.iter().flatten().copied().collect();
    let l = cholesky(&mut a, n);
    std::array::from_fn(|c| {
        let mut e = vec![0.0; n];
        e[c] = 1.0;
        solve_cholesky(&a, &l, n, &mut e);
        std::array::from_fn(|r| e[r])
    })
}

/** Factors `a` (n × n, row by row) in place as L Lᵀ; which pivots were too small to keep (left as nothing). */
fn cholesky(a: &mut [f64], n: usize) -> Vec<bool> {
    let largest = (0..n).map(|i| a[i * n + i]).fold(0.0, f64::max);
    let mut dropped = vec![false; n];
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d <= largest * 1e-12 {
            dropped[j] = true;
            for i in j..n {
                a[i * n + j] = 0.0;
            }
            continue;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    dropped
}

fn solve_cholesky(a: &[f64], dropped: &[bool], n: usize, b: &mut [f64]) {
    for i in 0..n {
        if dropped[i] {
            b[i] = 0.0;
            continue;
        }
        let s: f64 = (0..i).map(|k| a[i * n + k] * b[k]).sum();
        b[i] = (b[i] - s) / a[i * n + i];
    }
    for i in (0..n).rev() {
        if dropped[i] {
            b[i] = 0.0;
            continue;
        }
        let s: f64 = (i + 1..n).map(|k| a[k * n + i] * b[k]).sum();
        b[i] = (b[i] - s) / a[i * n + i];
    }
}

/** A coarser level: each finer part's part here and where it is from its middle; the parts' blocks. */
struct Coarse {
    of: Vec<u32>,
    r: Vec<V3>,
    keys: Vec<[i64; 3]>,
    centre: Vec<V3>,
    diag: Vec<M6>,
    pairs: Vec<(u32, u32, M6)>,
    inverse: Vec<M6>,
    /** At the top, all of it, factored. */
    direct: Option<(Vec<f64>, Vec<bool>)>,
    /** How far across a part is, metres. */
    size: f64,
    /** How far a smoothing step goes of what the diagonal asks (`step`). */
    step: f64,
}

/** The finer parts joined within each octree cell a level up (keys halved): each one's part, how many parts. */
fn aggregate(keys: &[[i64; 3]], pairs: impl Iterator<Item = (usize, usize)>) -> (Vec<u32>, usize) {
    let n = keys.len();
    let mut up: Vec<usize> = (0..n).collect();
    fn find(up: &mut [usize], mut i: usize) -> usize {
        while up[i] != i {
            up[i] = up[up[i]];
            i = up[i];
        }
        i
    }
    let parent = |k: [i64; 3]| k.map(|v| v.div_euclid(2));
    for (i, j) in pairs {
        if parent(keys[i]) == parent(keys[j]) {
            let (a, b) = (find(&mut up, i), find(&mut up, j));
            up[a.max(b)] = a.min(b);
        }
    }
    let mut index = vec![u32::MAX; n];
    let mut of = vec![0; n];
    let mut count = 0;
    for i in 0..n {
        let root = find(&mut up, i);
        if index[root] == u32::MAX {
            index[root] = count;
            count += 1;
        }
        of[i] = index[root];
    }
    (of, count as usize)
}

/** The next level up from finer parts: their keys, where they are, their blocks. */
fn coarsen(keys: &[[i64; 3]], at: &[V3], diag: &[M6], pairs: &[(u32, u32, M6)], size: f64) -> Coarse {
    let (of, n) = aggregate(keys, pairs.iter().map(|p| (p.0 as usize, p.1 as usize)));
    let mut centre = vec![[0.0; 3]; n];
    let mut count = vec![0.0; n];
    let mut ckeys = vec![[0; 3]; n];
    for (i, &a) in of.iter().enumerate() {
        centre[a as usize] = add(centre[a as usize], at[i]);
        count[a as usize] += 1.0;
        ckeys[a as usize] = keys[i].map(|v| v.div_euclid(2));
    }
    for a in 0..n {
        centre[a] = scale(centre[a], 1.0 / count[a]);
    }
    let r: Vec<V3> = of.iter().enumerate().map(|(i, &a)| sub(at[i], centre[a as usize])).collect();
    let mut cdiag = vec![[[0.0; 6]; 6]; n];
    for (i, d) in diag.iter().enumerate() {
        plus6(&mut cdiag[of[i] as usize], &lift(d, r[i], r[i]));
    }
    let mut map: lattice::FastMap<(u32, u32), M6> = lattice::FastMap::default();
    for &(i, j, ref b) in pairs {
        let (i, j) = (i as usize, j as usize);
        let (a, c) = (of[i], of[j]);
        let m = lift(b, r[i], r[j]);
        if a == c {
            plus6(&mut cdiag[a as usize], &m);
            plus6(&mut cdiag[a as usize], &transpose6(&m));
        } else if a < c {
            plus6(map.entry((a, c)).or_insert([[0.0; 6]; 6]), &m);
        } else {
            plus6(map.entry((c, a)).or_insert([[0.0; 6]; 6]), &transpose6(&m));
        }
    }
    let cpairs: Vec<(u32, u32, M6)> = map.into_iter().map(|((a, c), m)| (a, c, m)).collect();
    // A part's turn its cells do not reach (one cell, a line) is held by a little of what its translation is, at its size.
    let inverse = cdiag
        .iter()
        .map(|d| {
            let mut d = *d;
            let t = (d[0][0] + d[1][1] + d[2][2]) / 3.0;
            for k in 3..6 {
                d[k][k] += 1e-6 * t * size * size;
            }
            self::inverse(&d)
        })
        .collect();
    let mut coarse = Coarse { of, r, keys: ckeys, centre, diag: cdiag, pairs: cpairs, inverse, direct: None, size, step: 1.0 };
    coarse.step = step(n, |x, y| coarse.apply(x, y), |x, y| (0..n).for_each(|p| y[p] = times6(&coarse.inverse[p], &x[p])));
    if n <= DIRECT {
        coarse.factor();
    }
    coarse
}

/**
 * How far a smoothing step may go of what the diagonal asks: 4/3 over the
 * greatest eigenvalue of the matrix over its diagonal, by a few powers of
 * it (`apply`, then `over`), so that the smoother, and the V-cycle, is
 * symmetric and positive.
 */
fn step<const N: usize>(n: usize, apply: impl Fn(&[[f64; N]], &mut [[f64; N]]), over: impl Fn(&[[f64; N]], &mut [[f64; N]])) -> f64 {
    let norm = |v: &[[f64; N]]| v.iter().flatten().map(|x| x * x).sum::<f64>().sqrt();
    let mut v: Vec<[f64; N]> = (0..n).map(|k| std::array::from_fn(|s| 1.0 + ((7 * k + 3 * s) % 11) as f64 / 11.0)).collect();
    let (mut a, mut w) = (vec![[0.0; N]; n], vec![[0.0; N]; n]);
    let mut most = 1.0;
    for _ in 0..10 {
        apply(&v, &mut a);
        over(&a, &mut w);
        let (nw, nv) = (norm(&w), norm(&v));
        if nw == 0.0 {
            break;
        }
        most = nw / nv;
        v.iter_mut().zip(&w).for_each(|(v, w)| *v = w.map(|x| x / nw));
    }
    4.0 / 3.0 / most
}

impl Coarse {
    fn apply(&self, x: &[V6], y: &mut [V6]) {
        for (p, d) in self.diag.iter().enumerate() {
            y[p] = times6(d, &x[p]);
        }
        for &(p, q, ref m) in &self.pairs {
            let (p, q) = (p as usize, q as usize);
            let (a, c) = (times6(m, &x[q]), times6t(m, &x[p]));
            y[p] = std::array::from_fn(|k| y[p][k] + a[k]);
            y[q] = std::array::from_fn(|k| y[q][k] + c[k]);
        }
    }

    /** Factors all of it, to be solved directly. */
    fn factor(&mut self) {
        let n = self.centre.len();
        let size = self.size;
        let mut a = vec![0.0; 36 * n * n];
        let m = 6 * n;
        for (p, d) in self.diag.iter().enumerate() {
            for r in 0..6 {
                for c in 0..6 {
                    a[(6 * p + r) * m + 6 * p + c] += d[r][c];
                }
            }
            let t = (d[0][0] + d[1][1] + d[2][2]) / 3.0;
            for k in 3..6 {
                a[(6 * p + k) * m + 6 * p + k] += 1e-6 * t * size * size;
            }
        }
        for &(p, q, ref b) in &self.pairs {
            let (p, q) = (p as usize, q as usize);
            for r in 0..6 {
                for c in 0..6 {
                    a[(6 * p + r) * m + 6 * q + c] += b[r][c];
                    a[(6 * q + c) * m + 6 * p + r] += b[r][c];
                }
            }
        }
        let dropped = cholesky(&mut a, m);
        self.direct = Some((a, dropped));
    }
}

/**
 * The levels of a system: made for one, they serve any close to it (the
 * same cells, a little moved, contacts let go), solving it as well as
 * they fit it, never wrongly.
 */
pub struct Solver {
    inverse: Vec<M3>,
    step: f64,
    levels: Vec<Coarse>,
}

impl Solver {
    /** The levels of `system`, whose cells are `size` metres across. */
    pub fn new(system: &System, size: f64) -> Solver {
        let inverse: Vec<M3> = system.diag.iter().map(inverse).collect();
        let step = step(system.sites.len(), |x, y| system.times(x, y), |x, y| (0..x.len()).for_each(|i| y[i] = times(&inverse[i], x[i])));
        let mut levels: Vec<Coarse> = Vec::new();
        let diag: Vec<M6> = system.diag.iter().map(embed).collect();
        let pairs: Vec<(u32, u32, M6)> = system.pairs.iter().map(|&(i, j, ref b)| (i, j, embed(b))).collect();
        let mut size = size;
        let mut next = coarsen(&system.sites, &system.at, &diag, &pairs, size * 2.0);
        loop {
            size *= 2.0;
            let (n, before) = (next.centre.len(), levels.last().map_or(system.sites.len(), |l| l.centre.len()));
            if next.direct.is_some() {
                levels.push(next);
                break;
            }
            if n == before {
                // Pieces apart, each a part: few, solved directly.
                next.factor();
                levels.push(next);
                break;
            }
            levels.push(next);
            let l = levels.last().expect("a level");
            next = coarsen(&l.keys, &l.centre, &l.diag, &l.pairs, size * 2.0);
        }
        Solver { inverse, step, levels }
    }

    /** The coarse correction from level `l` (0 the first coarse one) for residual `b`. */
    fn cycle(&self, l: usize, b: &[V6]) -> Vec<V6> {
        let level = &self.levels[l];
        let n = level.centre.len();
        if let Some((a, dropped)) = &level.direct {
            let mut x: Vec<f64> = b.iter().flatten().copied().collect();
            solve_cholesky(a, dropped, 6 * n, &mut x);
            return x.chunks(6).map(|c| std::array::from_fn(|k| c[k])).collect();
        }
        let apply = |x: &[V6]| -> Vec<V6> {
            let mut y = vec![[0.0; 6]; n];
            level.apply(x, &mut y);
            y
        };
        let smooth = |x: &mut [V6], r: &[V6]| {
            for p in 0..n {
                let d = times6(&level.inverse[p], &r[p]);
                x[p] = std::array::from_fn(|k| x[p][k] + level.step * d[k]);
            }
        };
        let residual = |x: &[V6]| -> Vec<V6> { apply(x).iter().zip(b).map(|(y, b)| std::array::from_fn(|k| b[k] - y[k])).collect() };
        let mut x = vec![[0.0; 6]; n];
        smooth(&mut x, b);
        let r = residual(&x);
        let up = &self.levels[l + 1];
        let mut rb = vec![[0.0; 6]; up.centre.len()];
        for p in 0..n {
            let (a, rp) = (up.of[p] as usize, up.r[p]);
            let t = cross(rp, [r[p][0], r[p][1], r[p][2]]);
            for k in 0..3 {
                rb[a][k] += r[p][k];
                rb[a][3 + k] += r[p][3 + k] + t[k];
            }
        }
        let e = self.cycle(l + 1, &rb);
        for p in 0..n {
            let a = up.of[p] as usize;
            let u = cross([e[a][3], e[a][4], e[a][5]], up.r[p]);
            for k in 0..3 {
                x[p][k] += e[a][k] + u[k];
                x[p][3 + k] += e[a][3 + k];
            }
        }
        let r = residual(&x);
        smooth(&mut x, &r);
        x
    }

    /** The preconditioner: a V-cycle from the cells. */
    fn precondition(&self, s: &System, b: &[V3], out: &mut [V3], scratch: &mut [V3]) {
        let n = b.len();
        for i in 0..n {
            out[i] = scale(times(&self.inverse[i], b[i]), self.step);
        }
        s.times(out, scratch);
        let up = &self.levels[0];
        let mut rb = vec![[0.0; 6]; up.centre.len()];
        for i in 0..n {
            let r = sub(b[i], scratch[i]);
            let (a, t) = (up.of[i] as usize, cross(up.r[i], r));
            for k in 0..3 {
                rb[a][k] += r[k];
                rb[a][3 + k] += t[k];
            }
        }
        let e = self.cycle(0, &rb);
        for i in 0..n {
            let a = up.of[i] as usize;
            out[i] = add(out[i], add([e[a][0], e[a][1], e[a][2]], cross([e[a][3], e[a][4], e[a][5]], up.r[i])));
        }
        s.times(out, scratch);
        for i in 0..n {
            out[i] = add(out[i], scale(times(&self.inverse[i], sub(b[i], scratch[i])), self.step));
        }
    }

    /**
     * Solves `s` for `x`, from what it holds, till the residual is
     * `tolerance` of `b`, each measured by what the preconditioner makes of
     * it (rᵀ P r, an energy): so a light node is held to its speed as a
     * heavy one is, not to a share of every force at once. How many rounds
     * it took.
     */
    pub fn solve(&self, s: &System, b: &[V3], x: &mut [V3], tolerance: f64) -> usize {
        let n = b.len();
        if n == 0 {
            return 0;
        }
        let measure = |r: &[V3], z: &[V3]| r.iter().zip(z).map(|(r, z)| dot(*r, *z)).sum::<f64>();
        let (mut z, mut scratch) = (vec![[0.0; 3]; n], vec![[0.0; 3]; n]);
        self.precondition(s, b, &mut z, &mut scratch);
        let target = tolerance * tolerance * measure(b, &z);
        let mut ax = vec![[0.0; 3]; n];
        s.times(x, &mut ax);
        let mut r: Vec<V3> = b.iter().zip(&ax).map(|(b, a)| sub(*b, *a)).collect();
        self.precondition(s, &r, &mut z, &mut scratch);
        let mut p = z.clone();
        let mut rz = measure(&r, &z);
        let mut ap = vec![[0.0; 3]; n];
        for round in 0..ROUNDS {
            if rz <= target {
                return round;
            }
            s.times(&p, &mut ap);
            let pap: f64 = p.iter().zip(&ap).map(|(p, a)| dot(*p, *a)).sum();
            if pap <= 0.0 {
                return round;
            }
            let alpha = rz / pap;
            for i in 0..n {
                x[i] = add(x[i], scale(p[i], alpha));
                r[i] = sub(r[i], scale(ap[i], alpha));
            }
            self.precondition(s, &r, &mut z, &mut scratch);
            let next = measure(&r, &z);
            let beta = next / rz;
            rz = next;
            for i in 0..n {
                p[i] = add(z[i], scale(p[i], beta));
            }
        }
        ROUNDS
    }
}
