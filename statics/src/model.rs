//! Voxels and the bonds that join them: what one weighs, what one carries.
//!
//! A bond is a spring at a point of a face two voxels share, with a share
//! of its area: stiff along the face's normal as the material is, and
//! across it as the material is in shear. Four bonds to a face, placed so
//! that their area and second moments are the face's, so that faces carry
//! moments as they would whole: a line of voxels is a beam, not a chain.
//! Each voxel's stress is what its bonds' forces add up to over its
//! volume. The stress at a bond is its voxels', but on its own face what
//! it bears itself; its share of its material's strength is the greatest
//! of tension, compression and shear, whichever way the stress lies, so
//! that no direction of the lattice is stronger than another.

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
    /// The share of its strength a stress uses, 1 and over breaking it: its
    /// greatest tension, its greatest compression, its greatest shear.
    pub fn utilisation(&self, stress: &M3) -> f64 {
        let [s3, _, s1] = principal(stress);
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

/// A voxel: where its matter's centre is, what it weighs, whether it is
/// held (ground that never moves).
#[derive(Clone, Debug)]
pub struct Voxel {
    pub x: V3,
    pub m: f64,
    /// The volume of the cell it fills.
    pub volume: f64,
    /// What most of it is.
    pub mat: Mat,
    pub held: bool,
}

#[derive(Clone, Debug)]
pub struct Bond {
    pub i: usize,
    pub j: usize,
    /// Where it is, the face's normal from i to j, its share of the area,
    /// the distance between the voxels' centres.
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

    /// The stress at the bond, of voxels whose stress is `around` (on
    /// average), as it bears `force`: theirs, but on its face its own.
    pub fn stress(&self, around: &M3, force: V3) -> M3 {
        let n = self.n;
        let theirs = std::array::from_fn::<f64, 3, _>(|r| dot(around[r], n));
        let own = mul(force, 1.0 / self.a);
        let (a, b) = (dot(n, theirs), dot(n, own));
        std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                around[r][c] - n[r] * theirs[c] - theirs[r] * n[c] + a * n[r] * n[c] + n[r] * own[c] + own[r] * n[c] - b * n[r] * n[c]
            })
        })
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

/// The principal stresses of a symmetric stress, least first.
pub fn principal(s: &M3) -> [f64; 3] {
    let mean = (s[0][0] + s[1][1] + s[2][2]) / 3.0;
    let off = s[0][1].powi(2) + s[0][2].powi(2) + s[1][2].powi(2);
    let dev = (0..3).map(|k| (s[k][k] - mean).powi(2)).sum::<f64>() + 2.0 * off;
    let p = (dev / 6.0).sqrt();
    if p == 0.0 {
        return [mean; 3];
    }
    // The eigenvalues of s = mean + 2p·cos(φ + 2πk/3), φ from det((s − mean)/p) / 2.
    let b: M3 = std::array::from_fn(|r| std::array::from_fn(|c| (s[r][c] - if r == c { mean } else { 0.0 }) / p));
    let det = b[0][0] * (b[1][1] * b[2][2] - b[1][2] * b[2][1]) - b[0][1] * (b[1][0] * b[2][2] - b[1][2] * b[2][0])
        + b[0][2] * (b[1][0] * b[2][1] - b[1][1] * b[2][0]);
    let phi = (det / 2.0).clamp(-1.0, 1.0).acos() / 3.0;
    let third = 2.0 * std::f64::consts::PI / 3.0;
    let mut e = [0.0, 1.0, 2.0].map(|k| mean + 2.0 * p * (phi + k * third).cos());
    e.sort_by(f64::total_cmp);
    e
}
