//! Rigid voxels joined by springs on their shared faces (RBSM), statics only.
//! A node: a rigid body, 6 DOF. A link: a connection patch between two nodes,
//! with its own frame (n, p, q), area, second moments, and check points.

pub type V3 = [f64; 3];
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn unit(a: V3) -> V3 {
    mul(a, 1.0 / norm(a))
}

/// Stiffnesses of a face: translational and rotational.
pub type Stiff = ([[f64; 3]; 3], [[f64; 3]; 3]);

pub const G: f64 = 9.81;
pub const UNIT: f64 = 0.125;

#[derive(Clone, Copy, Debug)]
pub struct Mat {
    pub name: &'static str,
    pub rho: f64,
    pub e: f64,
    pub ft: f64,
    pub fc: f64,
    pub fs: f64,
}
pub const STONE: Mat = Mat { name: "stone", rho: 2500.0, e: 30e9, ft: 5e6, fc: 100e6, fs: 10e6 };
pub const WOOD: Mat = Mat { name: "wood", rho: 600.0, e: 10e9, ft: 40e6, fc: 30e6, fs: 6e6 };
pub const MASONRY: Mat = Mat { name: "masonry", rho: 2000.0, e: 5e9, ft: 0.2e6, fc: 10e6, fs: 0.4e6 };
pub const SOIL: Mat = Mat { name: "soil", rho: 1800.0, e: 50e6, ft: 5e3, fc: 0.3e6, fs: 20e3 };
pub const LEAVES: Mat = Mat { name: "leaves", rho: 60.0, e: 0.2e9, ft: 5e6, fc: 5e6, fs: 1e6 };

#[derive(Clone, Debug)]
pub struct Node {
    pub x: V3,
    pub m: f64,
    pub fixed: bool,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub i: usize,
    pub j: usize,
    pub c: V3,
    pub n: V3,
    pub p: V3,
    pub q: V3,
    pub d: f64,
    pub a: f64,
    /// ∫sq², ∫sp², ∫sp·sq over the patch, (sp, sq) coordinates along p, q from c.
    pub iqq: f64,
    pub ipp: f64,
    pub ipq: f64,
    /// Points on the patch where stress is checked, in (sp, sq).
    pub pts: Vec<[f64; 2]>,
    pub mat: Mat,
    pub broken: bool,
}

impl Link {
    fn shear(&self) -> f64 {
        0.4
    }
    /// Translational and rotational stiffness, global 3x3.
    pub fn kt(&self) -> [[f64; 3]; 3] {
        let kn = self.mat.e * self.a / self.d;
        let ks = self.shear() * kn;
        let mut k = [[0.0; 3]; 3];
        for r in 0..3 {
            for s in 0..3 {
                k[r][s] = kn * self.n[r] * self.n[s] + ks * (self.p[r] * self.p[s] + self.q[r] * self.q[s]);
            }
        }
        k
    }
    pub fn kr(&self) -> [[f64; 3]; 3] {
        let e = self.mat.e / self.d;
        let gt = self.shear() * e * (self.ipp + self.iqq);
        // opening at (sp,sq) = wp*sq - wq*sp; energy E/2d ∫ (wp sq - wq sp)^2
        let kpp = e * self.iqq;
        let kqq = e * self.ipp;
        let kpq = -e * self.ipq;
        let mut k = [[0.0; 3]; 3];
        for r in 0..3 {
            for s in 0..3 {
                k[r][s] = kpp * self.p[r] * self.p[s]
                    + kqq * self.q[r] * self.q[s]
                    + kpq * (self.p[r] * self.q[s] + self.q[r] * self.p[s])
                    + gt * self.n[r] * self.n[s];
            }
        }
        k
    }
}

pub fn mv(k: &[[f64; 3]; 3], v: V3) -> V3 {
    [dot(k[0], v), dot(k[1], v), dot(k[2], v)]
}

pub struct Structure {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
}

/// Relative motion of j against i at the patch: (translation at c, rotation).
pub fn rel(l: &Link, nodes: &[Node], x: &[[f64; 6]]) -> (V3, V3) {
    let xi = x[l.i];
    let xj = x[l.j];
    let ri = sub(l.c, nodes[l.i].x);
    let rj = sub(l.c, nodes[l.j].x);
    let ui = add([xi[0], xi[1], xi[2]], cross([xi[3], xi[4], xi[5]], ri));
    let uj = add([xj[0], xj[1], xj[2]], cross([xj[3], xj[4], xj[5]], rj));
    (sub(uj, ui), sub([xj[3], xj[4], xj[5]], [xi[3], xi[4], xi[5]]))
}

/// z = M⁻¹ r.
pub type Precond<'a> = &'a dyn Fn(&[[f64; 6]], &mut [[f64; 6]]);

pub struct Solution {
    pub x: Vec<[f64; 6]>,
    pub iters: usize,
    pub converged: bool,
}

impl Structure {
    pub fn live(&self) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(|l| !l.broken)
    }

    fn apply(&self, x: &[[f64; 6]], y: &mut [[f64; 6]], kts: &[Stiff]) {
        for v in y.iter_mut() {
            *v = [0.0; 6];
        }
        for (l, (kt, kr)) in self.links.iter().zip(kts) {
            if l.broken {
                continue;
            }
            let (dt, dr) = rel(l, &self.nodes, x);
            let f = mv(kt, dt);
            let m = mv(kr, dr);
            for (k, s) in [(l.j, 1.0), (l.i, -1.0)] {
                if self.nodes[k].fixed {
                    continue;
                }
                let r = sub(l.c, self.nodes[k].x);
                let t = add(cross(r, f), m);
                for a in 0..3 {
                    y[k][a] += s * f[a];
                    y[k][3 + a] += s * t[a];
                }
            }
        }
    }

    fn block_diag(&self, kts: &[Stiff]) -> Vec<[[f64; 6]; 6]> {
        let mut d = vec![[[0.0; 6]; 6]; self.nodes.len()];
        for (l, (kt, kr)) in self.links.iter().zip(kts) {
            if l.broken {
                continue;
            }
            for k in [l.i, l.j] {
                let r = sub(l.c, self.nodes[k].x);
                // B = [I, -[r]x]  (translation at c = u + θ×r = u - r×θ)
                let mut b = [[0.0; 6]; 3];
                for a in 0..3 {
                    b[a][a] = 1.0;
                }
                // -[r]x
                b[0][4] = r[2];
                b[0][5] = -r[1];
                b[1][3] = -r[2];
                b[1][5] = r[0];
                b[2][3] = r[1];
                b[2][4] = -r[0];
                for s in 0..6 {
                    for t in 0..6 {
                        let mut v = 0.0;
                        for a in 0..3 {
                            for c in 0..3 {
                                v += b[a][s] * kt[a][c] * b[c][t];
                            }
                        }
                        if s >= 3 && t >= 3 {
                            v += kr[s - 3][t - 3];
                        }
                        d[k][s][t] += v;
                    }
                }
            }
        }
        d
    }

    pub fn loads(&self) -> Vec<[f64; 6]> {
        self.nodes.iter().map(|n| if n.fixed { [0.0; 6] } else { [0.0, -n.m * G, 0.0, 0.0, 0.0, 0.0] }).collect()
    }

    pub fn block_inverse(&self) -> Vec<[[f64; 6]; 6]> {
        let kts: Vec<_> = self.links.iter().map(|l| (l.kt(), l.kr())).collect();
        self.block_diag(&kts).iter().map(inv6).collect()
    }

    pub fn solve(&self, f: &[[f64; 6]], warm: Option<&[[f64; 6]]>, tol: f64, max_iter: usize) -> Solution {
        let inv = self.block_inverse();
        let jacobi = |r: &[[f64; 6]], z: &mut [[f64; 6]]| {
            for k in 0..r.len() {
                z[k] = mat6(&inv[k], r[k]);
            }
        };
        self.solve_with(f, warm, tol, max_iter, &jacobi)
    }

    pub fn solve_with(&self, f: &[[f64; 6]], warm: Option<&[[f64; 6]]>, tol: f64, max_iter: usize, precond: Precond) -> Solution {
        let n = self.nodes.len();
        let kts: Vec<_> = self.links.iter().map(|l| (l.kt(), l.kr())).collect();
        let mut x = warm.map(|w| w.to_vec()).unwrap_or_else(|| vec![[0.0; 6]; n]);
        for (k, nd) in self.nodes.iter().enumerate() {
            if nd.fixed {
                x[k] = [0.0; 6];
            }
        }
        let mut ax = vec![[0.0; 6]; n];
        self.apply(&x, &mut ax, &kts);
        let mut r: Vec<[f64; 6]> = (0..n).map(|k| sub6(f[k], ax[k])).collect();
        let fnorm = dot6s(f, f).sqrt().max(1e-30);
        let prec = |r: &[[f64; 6]], z: &mut Vec<[f64; 6]>| {
            precond(r, z);
            for k in 0..n {
                if self.nodes[k].fixed {
                    z[k] = [0.0; 6];
                }
            }
        };
        let mut z = vec![[0.0; 6]; n];
        prec(&r, &mut z);
        let mut p = z.clone();
        let mut rz = dot6s(&r, &z);
        let mut iters = 0;
        let mut converged = false;
        while iters < max_iter {
            if dot6s(&r, &r).sqrt() <= tol * fnorm {
                converged = true;
                break;
            }
            self.apply(&p, &mut ax, &kts);
            let pap = dot6s(&p, &ax);
            if pap <= 0.0 {
                break;
            }
            let alpha = rz / pap;
            for k in 0..n {
                for a in 0..6 {
                    x[k][a] += alpha * p[k][a];
                    r[k][a] -= alpha * ax[k][a];
                }
            }
            prec(&r, &mut z);
            let rz2 = dot6s(&r, &z);
            let beta = rz2 / rz;
            rz = rz2;
            for k in 0..n {
                for a in 0..6 {
                    p[k][a] = z[k][a] + beta * p[k][a];
                }
            }
            iters += 1;
        }
        Solution { x, iters, converged }
    }

    /// Utilisation of each link: the worst of tension, compression, shear.
    pub fn utilisation(&self, x: &[[f64; 6]]) -> Vec<f64> {
        self.links
            .iter()
            .map(|l| {
                if l.broken {
                    return 0.0;
                }
                let (dt, dr) = rel(l, &self.nodes, x);
                let e = l.mat.e / l.d;
                let open = dot(dt, l.n);
                let wp = dot(dr, l.p);
                let wq = dot(dr, l.q);
                let mut st: f64 = f64::MIN;
                let mut sc: f64 = f64::MAX;
                for pt in &l.pts {
                    let s = e * (open + wp * pt[1] - wq * pt[0]);
                    st = st.max(s);
                    sc = sc.min(s);
                }
                let shear = l.shear() * e * (dot(dt, l.p).powi(2) + dot(dt, l.q).powi(2)).sqrt();
                (st / l.mat.ft).max(-sc / l.mat.fc).max(shear / l.mat.fs)
            })
            .collect()
    }

    /// Components; which reach a fixed node.
    pub fn components(&self) -> (Vec<usize>, Vec<bool>) {
        let n = self.nodes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut a: usize) -> usize {
            while p[a] != a {
                p[a] = p[p[a]];
                a = p[a];
            }
            a
        }
        for l in self.live() {
            let (a, b) = (find(&mut parent, l.i), find(&mut parent, l.j));
            if a != b {
                parent[a] = b;
            }
        }
        let roots: Vec<usize> = (0..n).map(|k| find(&mut parent, k)).collect();
        let mut grounded = vec![false; n];
        for k in 0..n {
            if self.nodes[k].fixed {
                grounded[roots[k]] = true;
            }
        }
        let g = (0..n).map(|k| grounded[roots[k]]).collect();
        (roots, g)
    }
}

pub struct Outcome {
    pub fallen_mass: f64,
    pub total_mass: f64,
    pub broken: usize,
    pub rounds: usize,
    pub iters: usize,
    pub first_umax: f64,
}

/// Solve, break the links over their strength (the worst first), drop what
/// is no longer held, again, until what is left holds.
pub fn settle(s: &mut Structure, verbose: bool) -> Outcome {
    let total_mass: f64 = s.nodes.iter().filter(|n| !n.fixed).map(|n| n.m).sum();
    let mut fallen = vec![false; s.nodes.len()];
    let mut warm: Option<Vec<[f64; 6]>> = None;
    let mut rounds = 0;
    let mut iters = 0;
    let mut broken = 0;
    let mut first_umax = f64::NAN;
    loop {
        // Drop what hangs on nothing.
        let (_, grounded) = s.components();
        for k in 0..s.nodes.len() {
            if !grounded[k] && !fallen[k] {
                fallen[k] = true;
            }
        }
        for l in s.links.iter_mut() {
            if fallen[l.i] || fallen[l.j] {
                l.broken = true;
            }
        }
        let mut f = s.loads();
        for k in 0..s.nodes.len() {
            if fallen[k] {
                f[k] = [0.0; 6];
            }
        }
        let sol = s.solve(&f, warm.as_deref(), 1e-7, 200_000);
        iters += sol.iters;
        rounds += 1;
        let u = s.utilisation(&sol.x);
        let umax = u.iter().cloned().fold(0.0, f64::max);
        if first_umax.is_nan() {
            first_umax = umax;
        }
        if verbose {
            eprintln!("  round {rounds}: iters {} conv {} umax {umax:.3}", sol.iters, sol.converged);
        }
        if umax <= 1.0 {
            break;
        }
        let frac: f64 = std::env::var("BREAK").ok().and_then(|v| v.parse().ok()).unwrap_or(0.95);
        let cut = (frac * umax).max(1.0);
        for (l, &ul) in s.links.iter_mut().zip(&u) {
            if ul > cut {
                l.broken = true;
                broken += 1;
            }
        }
        warm = Some(sol.x);
        if rounds > 2000 {
            break;
        }
    }
    let fallen_mass = s.nodes.iter().zip(&fallen).filter(|(_, f)| **f).map(|(n, _)| n.m).sum();
    Outcome { fallen_mass, total_mass, broken, rounds, iters, first_umax }
}

fn sub6(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    let mut r = [0.0; 6];
    for k in 0..6 {
        r[k] = a[k] - b[k];
    }
    r
}
fn dot6s(a: &[[f64; 6]], b: &[[f64; 6]]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (0..6).map(|k| x[k] * y[k]).sum::<f64>()).sum()
}
pub fn mat6(m: &[[f64; 6]; 6], v: [f64; 6]) -> [f64; 6] {
    let mut r = [0.0; 6];
    for a in 0..6 {
        for b in 0..6 {
            r[a] += m[a][b] * v[b];
        }
    }
    r
}
fn inv6(m: &[[f64; 6]; 6]) -> [[f64; 6]; 6] {
    let mut a = *m;
    let mut inv = [[0.0; 6]; 6];
    for k in 0..6 {
        inv[k][k] = 1.0;
    }
    let scale = (0..6).map(|k| a[k][k].abs()).fold(0.0, f64::max).max(1e-300);
    for col in 0..6 {
        let piv = (col..6).max_by(|&x, &y| a[x][col].abs().partial_cmp(&a[y][col].abs()).unwrap()).unwrap();
        if a[piv][col].abs() < 1e-14 * scale {
            // Singular (a fixed node, or a lone one): leave it out.
            return [[0.0; 6]; 6];
        }
        a.swap(col, piv);
        inv.swap(col, piv);
        let d = a[col][col];
        for k in 0..6 {
            a[col][k] /= d;
            inv[col][k] /= d;
        }
        for r in 0..6 {
            if r != col {
                let f = a[r][col];
                if f != 0.0 {
                    for k in 0..6 {
                        a[r][k] -= f * a[col][k];
                        inv[r][k] -= f * inv[col][k];
                    }
                }
            }
        }
    }
    inv
}
