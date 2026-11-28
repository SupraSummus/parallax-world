//! A cell as a finite element: a cube of eight nodes at its corners,
//! trilinear within, its stiffness integrated at 2×2×2 Gauss points, of
//! a material whose Poisson's ratio is NU. Computed once for a cube a
//! metre across of stiffness 1 (`Unit`): a cube h across of stiffness E is
//! E h times as stiff. Co-rotated: the cube is taken turned as its corners
//! are (`Element::follow`, by Müller et al.'s rotation extraction, from
//! the turn it had last), and bends in that turned frame, so it turns
//! freely however far.

use std::sync::OnceLock;

use crate::solve::{M3, V3, add, cross, dot, scale, sub, times, times_t};

/** Of every material: its Poisson's ratio. */
const NU: f64 = 0.3;

/** The corners of a cube, as steps from its least: corner c is c's bits x, y, z. */
pub const CORNERS: [[i64; 3]; 8] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]];

/** A corner as −1 or 1 each way. */
fn sign(c: usize) -> V3 {
    CORNERS[c].map(|b| (2 * b - 1) as f64)
}

/** The strain (xx, yy, zz, 2yz, 2xz, 2xy) a node's motion makes, of a unit cube, at a point of it (each way −1 to 1). */
fn strain(a: usize, at: V3) -> [V3; 6] {
    let s = sign(a);
    // ∂N/∂x of N = Π (1 + s ξ) / 8, ξ = 2 x.
    let g: V3 = std::array::from_fn(|k| s[k] / 4.0 * (0..3).filter(|&j| j != k).map(|j| 1.0 + s[j] * at[j]).product::<f64>());
    [[g[0], 0.0, 0.0], [0.0, g[1], 0.0], [0.0, 0.0, g[2]], [0.0, g[2], g[1]], [g[2], 0.0, g[0]], [g[1], g[0], 0.0]]
}

/** Stress of strain, of stiffness 1 (Voigt). */
fn stress_of(e: &[f64; 6]) -> [f64; 6] {
    let (l, m) = (NU / ((1.0 + NU) * (1.0 - 2.0 * NU)), 1.0 / (2.0 * (1.0 + NU)));
    let tr = e[0] + e[1] + e[2];
    [l * tr + 2.0 * m * e[0], l * tr + 2.0 * m * e[1], l * tr + 2.0 * m * e[2], m * e[3], m * e[4], m * e[5]]
}

/** The unit cube: its stiffness, node by node; each corner's strain of each node's motion. */
struct Unit {
    k: [[M3; 8]; 8],
    at: [[[V3; 6]; 8]; 8],
}

fn unit() -> &'static Unit {
    static UNIT: OnceLock<Unit> = OnceLock::new();
    UNIT.get_or_init(|| {
        let g = 1.0 / 3f64.sqrt();
        let mut k = [[[[0.0; 3]; 3]; 8]; 8];
        for p in 0..8 {
            let at = sign(p).map(|s| s * g);
            let b: Vec<[V3; 6]> = (0..8).map(|a| strain(a, at)).collect();
            for a in 0..8 {
                for c in 0..8 {
                    for j in 0..3 {
                        let e: [f64; 6] = std::array::from_fn(|r| b[c][r][j]);
                        let s = stress_of(&e);
                        for i in 0..3 {
                            // Each point weighs an eighth of the cube.
                            k[a][c][i][j] += (0..6).map(|r| b[a][r][i] * s[r]).sum::<f64>() / 8.0;
                        }
                    }
                }
            }
        }
        let at = std::array::from_fn(|c| std::array::from_fn(|a| strain(a, sign(c))));
        Unit { k, at }
    })
}

fn column(m: &M3, k: usize) -> V3 {
    [m[0][k], m[1][k], m[2][k]]
}

/** The turn by `w`, about it by as many radians as it is long, as a unit quaternion. */
pub fn turn(w: V3) -> [f64; 4] {
    let angle = dot(w, w).sqrt();
    // sin(a / 2) / a, which is 1/2 as a is nothing.
    let s = if angle > 1e-12 { (angle / 2.0).sin() / angle } else { 0.5 };
    [(angle / 2.0).cos(), w[0] * s, w[1] * s, w[2] * s]
}

/** A unit quaternion (w, x, y, z) as a matrix. */
pub fn matrix([w, x, y, z]: [f64; 4]) -> M3 {
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/** The turn nearest `a`, from `q` (Müller, Bender, Chentanez and Macklin, 2016). */
fn rotation(a: &M3, mut q: [f64; 4]) -> [f64; 4] {
    for _ in 0..16 {
        let r = matrix(q);
        let (mut num, mut den) = ([0.0; 3], 0.0);
        for k in 0..3 {
            num = add(num, cross(column(&r, k), column(a, k)));
            den += dot(column(&r, k), column(a, k));
        }
        let omega = scale(num, 1.0 / (den.abs() + 1e-9));
        let w = dot(omega, omega).sqrt();
        if w < 1e-9 {
            break;
        }
        let d = turn(omega);
        let p = [
            d[0] * q[0] - d[1] * q[1] - d[2] * q[2] - d[3] * q[3],
            d[0] * q[1] + d[1] * q[0] + d[2] * q[3] - d[3] * q[2],
            d[0] * q[2] - d[1] * q[3] + d[2] * q[0] + d[3] * q[1],
            d[0] * q[3] + d[1] * q[2] - d[2] * q[1] + d[3] * q[0],
        ];
        let n = p.iter().map(|x| x * x).sum::<f64>().sqrt();
        q = p.map(|x| x / n);
    }
    q
}

/** A cell as its corners have it: its turn, and how each corner has moved in the turned frame, metres. */
#[derive(Clone, Debug)]
pub struct Element {
    q: [f64; 4],
    r: M3,
    u: [V3; 8],
}

impl Default for Element {
    fn default() -> Element {
        Element { q: [1.0, 0.0, 0.0, 0.0], r: matrix([1.0, 0.0, 0.0, 0.0]), u: [[0.0; 3]; 8] }
    }
}

impl Element {
    /** Takes the cell, `h` across, as its corners are at `x`. */
    pub fn follow(&mut self, x: &[V3; 8], h: f64) {
        // The deformation at the middle: F = Σ xₐ ⊗ ∇Nₐ, ∇Nₐ = sₐ / 4h.
        let f: M3 = std::array::from_fn(|r| std::array::from_fn(|c| (0..8).map(|a| x[a][r] * sign(a)[c]).sum::<f64>() / (4.0 * h)));
        self.q = rotation(&f, self.q);
        self.r = matrix(self.q);
        let middle = x.iter().fold([0.0; 3], |m, p| add(m, scale(*p, 0.125)));
        // Where each corner is in the turned frame, from where it would be: its motion there, translation and all.
        self.u = std::array::from_fn(|a| sub(times_t(&self.r, sub(x[a], middle)), scale(sign(a), h / 2.0)));
    }

    /** What holds each node back, of a cube a metre across and of stiffness 1, turned back. */
    pub fn forces(&self) -> [V3; 8] {
        let k = &unit().k;
        std::array::from_fn(|a| {
            let local = (0..8).fold([0.0; 3], |f, c| add(f, times(&k[a][c], self.u[c])));
            times(&self.r, local)
        })
    }

    /** The stiffness between nodes `a` and `c`, of a cube a metre across and of stiffness 1, turned: R Kₐ꜀ Rᵀ. */
    pub fn stiffness(&self, a: usize, c: usize) -> M3 {
        let (k, r) = (&unit().k[a][c], &self.r);
        std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|p| (0..3).map(|q| r[i][p] * k[p][q] * r[j][q]).sum::<f64>()).sum()))
    }

    /** The stress at corner `c`, of stiffness 1, in the cell's own frame, of a cell `h` across. */
    pub fn stress(&self, c: usize, h: f64) -> M3 {
        let b = &unit().at[c];
        let e: [f64; 6] = std::array::from_fn(|r| (0..8).map(|a| dot(b[a][r], self.u[a])).sum::<f64>() / h);
        let s = stress_of(&e);
        [[s[0], s[5], s[4]], [s[5], s[1], s[3]], [s[4], s[3], s[2]]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** The unit cube moved rigidly, by a translation or a turn, holds nothing back; stretched, as stiff as E along. */
    #[test]
    fn a_cube_turned_bears_nothing_and_stretched_is_as_stiff_as_its_material() {
        let home: [V3; 8] = std::array::from_fn(|a| CORNERS[a].map(|b| b as f64));
        let turn = matrix([0.8f64.cos(), 0.8f64.sin() * 0.6, 0.0, 0.8f64.sin() * 0.8]);
        let mut e = Element::default();
        e.follow(&home.map(|p| add(times(&turn, p), [3.0, -1.0, 2.0])), 1.0);
        assert!(e.forces().iter().all(|f| dot(*f, *f) < 1e-20), "{:?}", e.forces());
        // Pulled along x by a thousandth, free across: the force on the far face, a stress over its area 1.
        let mut e = Element::default();
        e.follow(&home.map(|p| [p[0] * 1.001, p[1] * (1.0 - NU * 0.001), p[2] * (1.0 - NU * 0.001)]), 1.0);
        let pull: f64 = (0..8).filter(|&a| CORNERS[a][0] == 1).map(|a| e.forces()[a][0]).sum();
        assert!((pull / 0.001 - 1.0).abs() < 1e-6, "{pull}");
        assert!((e.stress(7, 1.0)[0][0] / 0.001 - 1.0).abs() < 1e-6);
    }
}
