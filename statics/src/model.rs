//! Nodes and the bonds that join them: what one weighs, what one carries.
//!
//! A node is a face between two cells of a level; each cell joins its
//! faces by beams, and a beam is four bonds: springs at points of its
//! section, stiff along the beam as the material is, across it as the
//! material is in shear, placed so that their area and second moments
//! are the section's: a beam bears a moment, not only a pull. What a bond
//! bears over its area is the stress on its section; its share of its
//! material's strength is the greatest of tension, compression and shear,
//! whichever way that stress lies.

pub type V3 = [f64; 3];
/// Force and moment, or translation and rotation.
pub type Six = [f64; 6];

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

pub const GRAVITY: f64 = 9.81;

/// A material, in kg/m³ and pascals: density, stiffness along and across a
/// face, strength in tension, compression and shear.
#[derive(Clone, Copy, Debug)]
pub struct Mat {
    pub name: &'static str,
    pub rho: f64,
    pub e: f64,
    pub g: f64,
    pub ft: f64,
    pub fc: f64,
    pub fs: f64,
}

pub const STONE: Mat = Mat { name: "stone", rho: 2500.0, e: 30e9, g: 12e9, ft: 5e6, fc: 100e6, fs: 10e6 };
pub const WOOD: Mat = Mat { name: "wood", rho: 600.0, e: 10e9, g: 4e9, ft: 40e6, fc: 30e6, fs: 6e6 };
pub const MASONRY: Mat = Mat { name: "masonry", rho: 2000.0, e: 5e9, g: 2e9, ft: 0.2e6, fc: 10e6, fs: 0.4e6 };
pub const SOIL: Mat = Mat { name: "soil", rho: 1800.0, e: 50e6, g: 20e6, ft: 5e3, fc: 0.3e6, fs: 20e3 };
pub const STEEL: Mat = Mat { name: "steel", rho: 7850.0, e: 200e9, g: 80e9, ft: 250e6, fc: 250e6, fs: 145e6 };
pub const LEAVES: Mat = Mat { name: "leaves", rho: 60.0, e: 0.2e9, g: 0.08e9, ft: 5e6, fc: 5e6, fs: 1e6 };

pub type M3 = [[f64; 3]; 3];

impl Mat {
    /// The share of its strength a stress uses, 1 and over breaking it, of
    /// its greatest and least principal stresses: its greatest tension,
    /// its greatest compression, its greatest shear.
    pub fn used(&self, s1: f64, s3: f64) -> f64 {
        (s1 / self.ft).max(-s3 / self.fc).max((s1 - s3) / (2.0 * self.fs))
    }

    /// The same, `by` times as strong.
    pub fn stronger(self, by: f64) -> Mat {
        Mat { ft: self.ft * by, fc: self.fc * by, fs: self.fs * by, ..self }
    }

    /// Where two materials meet, the weaker of each.
    pub fn meet(self, o: Mat) -> Mat {
        Mat {
            name: if self.ft <= o.ft { self.name } else { o.name },
            rho: 0.0,
            e: self.e.min(o.e),
            g: self.g.min(o.g),
            ft: self.ft.min(o.ft),
            fc: self.fc.min(o.fc),
            fs: self.fs.min(o.fs),
        }
    }
}

/// A node: where it is, what weighs on it, whether it is held (ground
/// that never moves).
#[derive(Clone, Debug)]
pub struct Node {
    pub x: V3,
    pub m: f64,
    /// What most of the cells it lies between is.
    pub mat: Mat,
    pub held: bool,
}

#[derive(Clone, Debug)]
pub struct Bond {
    pub i: usize,
    pub j: usize,
    /// Where it is, the beam's direction from i to j, its share of the
    /// section, the length of the beam.
    pub p: V3,
    pub n: V3,
    pub a: f64,
    pub d: f64,
    pub mat: Mat,
}

impl Bond {
    /// The force j's side bears for moving by `delta` against i's side.
    pub fn force(&self, delta: V3) -> V3 {
        let along = dot(delta, self.n);
        let k = self.a / self.d;
        add(mul(self.n, self.mat.e * k * along), mul(sub(delta, mul(self.n, along)), self.mat.g * k))
    }

    /// The share of its strength it uses bearing `force`. A pull σ and a
    /// shear τ on its section: principal stresses σ/2 ± √(σ²/4 + τ²), and 0.
    pub fn used(&self, force: V3) -> f64 {
        let t = mul(force, 1.0 / self.a);
        let sigma = dot(t, self.n);
        let tau2 = (dot(t, t) - sigma * sigma).max(0.0);
        let r = (sigma * sigma / 4.0 + tau2).sqrt();
        self.mat.used(sigma / 2.0 + r, sigma / 2.0 - r)
    }

    /// The work stored in it as it bears `force`.
    pub fn energy(&self, force: V3) -> f64 {
        let k = self.a / self.d;
        let along = dot(force, self.n);
        0.5 * (along * along / (self.mat.e * k) + (dot(force, force) - along * along) / (self.mat.g * k))
    }

    /// The stiffness `force` is, as a matrix.
    pub fn stiffness(&self) -> M3 {
        let k = self.a / self.d;
        let mut s = [[0.0; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                let nn = self.n[r] * self.n[c];
                s[r][c] = self.mat.e * k * nn + self.mat.g * k * (if r == c { 1.0 } else { 0.0 } - nn);
            }
        }
        s
    }
}
