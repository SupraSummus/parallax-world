//! The faces the chunks are seen along: 18 directions, the cube's 6 axes
//! and its 12 edges' (docs/architecture.md, section 3), and the lattice
//! seen along each: layers across it, columns along it.

/** How many faces there are. */
pub const FACES: usize = 18;

/**
 * How far an axis face's rays are from its axis, as a tangent, at most,
 * across and up: tan 22.5°, where an edge face's rays begin. An edge
 * face's go as far across, and up as far as 1/√2.
 */
pub const TAN: f64 = std::f64::consts::SQRT_2 - 1.0;

/**
 * A face: a rotation, its lattice vectors R (right), U (up) and F
 * (forward), with R and F as long as each other, 1 (an axis face) or √2
 * (an edge face), and U one. Site p is at lattice coordinates (a, b, c) =
 * (p · R, p · U, p · F), at (a · layer, b, c · layer) in the face's axes:
 * layer c is the sites with a ≡ c (mod k), k = |F|², c · layer along the
 * axis, and column (a, b) the sites k layers apart along it.
 */
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Face {
    pub r: [i32; 3],
    pub u: [i32; 3],
    pub f: [i32; 3],
}

fn dot(a: [i32; 3], b: [i32; 3]) -> i32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/** The faces: the 6 along the axes, then the 12 along the edges, each up along +y; along y, +z; along an edge with y, the axis across it. */
pub fn faces() -> [Face; FACES] {
    let mut forwards = Vec::new();
    for d in 0..3 {
        for s in [1, -1] {
            let mut f = [0; 3];
            f[d] = s;
            forwards.push(f);
        }
    }
    for zero in 0..3 {
        let (i, j) = ((zero + 1) % 3, (zero + 2) % 3);
        for (si, sj) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let mut f = [0; 3];
            (f[i], f[j]) = (si, sj);
            forwards.push(f);
        }
    }
    std::array::from_fn(|n| {
        let f = forwards[n];
        let u = if f[1] == 0 {
            [0, 1, 0]
        } else if f[0] == 0 && f[2] == 0 {
            [0, 0, 1]
        } else {
            let mut u = [0; 3];
            u[(0..3).find(|&d| f[d] == 0).expect("an axis across")] = 1;
            u
        };
        Face { r: cross(u, f), u, f }
    })
}

impl Face {
    /** k = |F|²: 1 along an axis, 2 along an edge. */
    pub fn k(&self) -> i32 {
        dot(self.f, self.f)
    }

    /** The spacing of its layers, and of its columns across: 1 / √k. */
    pub fn layer(&self) -> f64 {
        1.0 / f64::from(self.k()).sqrt()
    }

    /** Site p's lattice coordinates (a, b, c). */
    pub fn of(&self, p: [i32; 3]) -> [i32; 3] {
        [dot(p, self.r), dot(p, self.u), dot(p, self.f)]
    }

    /** The site at lattice coordinates (a, b, c), a ≡ c (mod k): (a R + c F) / k + b U. */
    pub fn world(&self, [a, b, c]: [i32; 3]) -> [i32; 3] {
        let k = self.k();
        std::array::from_fn(|d| (a * self.r[d] + c * self.f[d]) / k + b * self.u[d])
    }

    /** A point at (x, y, z) in the face's axes, in the world. */
    pub fn point(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let l = self.layer();
        std::array::from_fn(|d| (x * f64::from(self.r[d]) + z * f64::from(self.f[d])) * l + y * f64::from(self.u[d]))
    }

    /** A point of the world in the face's axes. */
    pub fn axes_of(&self, p: [f64; 3]) -> [f64; 3] {
        let l = self.layer();
        let along = |v: [i32; 3]| p[0] * f64::from(v[0]) + p[1] * f64::from(v[1]) + p[2] * f64::from(v[2]);
        [along(self.r) * l, along(self.u), along(self.f) * l]
    }

    /**
     * Where a site's middle is past its lattice coordinates: site p is the
     * cube [p, p + 1), its middle p + ½, so (ΣR, ΣU, ΣF) / 2 more.
     */
    pub fn middle(&self) -> [f64; 3] {
        [self.r, self.u, self.f].map(|v| f64::from(v[0] + v[1] + v[2]) / 2.0)
    }

    /** How far its rays are from its axis, as tangents across and up, at most. */
    pub fn max_slant(&self) -> [f64; 2] {
        if self.k() == 1 { [TAN, TAN] } else { [TAN, std::f64::consts::FRAC_1_SQRT_2] }
    }

    /**
     * Whether a ray (x, y, z) in its axes is its own: nearer its axis than
     * any other face's. Along an axis, |x| and |y| ≤ TAN z; along an edge,
     * |x| ≤ TAN z and |x| + √2 |y| ≤ z.
     */
    pub fn holds(&self, [x, y, z]: [f64; 3]) -> bool {
        if self.k() == 1 {
            x.abs() <= TAN * z && y.abs() <= TAN * z
        } else {
            x.abs() <= TAN * z && x.abs() + std::f64::consts::SQRT_2 * y.abs() <= z
        }
    }

    /** The corners of its rays' tangents (x / z, y / z), round them. */
    pub fn corners(&self) -> Vec<[f64; 2]> {
        if self.k() == 1 {
            vec![[TAN, TAN], [-TAN, TAN], [-TAN, -TAN], [TAN, -TAN]]
        } else {
            let (h, y) = ((1.0 - TAN) * std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2);
            vec![[TAN, h], [0.0, y], [-TAN, h], [-TAN, -h], [0.0, -y], [TAN, -h]]
        }
    }

    /** The planes through the eye that cut its rays out, n · (x, y, z) ≥ 0 within: 4 along an axis, 6 along an edge. */
    pub fn planes(&self) -> Vec<[f64; 3]> {
        let s = std::f64::consts::SQRT_2;
        let mut planes = vec![[-1.0, 0.0, TAN], [1.0, 0.0, TAN]];
        if self.k() == 1 {
            planes.extend([[0.0, -1.0, TAN], [0.0, 1.0, TAN]]);
        } else {
            planes.extend([[-1.0, -s, 1.0], [-1.0, s, 1.0], [1.0, -s, 1.0], [1.0, s, 1.0]]);
        }
        planes
    }
}

/** N = 2^level, the spacing in sites of grid 0 of the cells of `level`. */
pub fn spacing(level: i32) -> i32 {
    1 << level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_rotations_one_along_each_axis_and_edge() {
        let faces = faces();
        let mut forwards: Vec<[i32; 3]> = faces.iter().map(|f| f.f).collect();
        forwards.sort();
        forwards.dedup();
        assert_eq!(forwards.len(), FACES);
        for face in &faces {
            let k = face.k();
            assert!(k == 1 || k == 2);
            assert_eq!(dot(face.r, face.r), k);
            assert_eq!(dot(face.u, face.u), 1);
            assert_eq!(dot(face.r, face.u) | dot(face.u, face.f) | dot(face.r, face.f), 0);
            // Right × up is forward: not mirrored.
            assert_eq!(cross(face.r, face.u), face.f);
            // Up is +y, or along y, +z.
            assert!(face.u == [0, 1, 0] || face.u == [0, 0, 1] || face.u == [1, 0, 0], "{face:?}");
        }
    }

    #[test]
    fn a_site_goes_to_its_lattice_coordinates_and_back() {
        for face in faces() {
            for p in [[0, 0, 0], [3, -2, 7], [-5, 1, 1], [1, 0, 0]] {
                let abc = face.of(p);
                assert_eq!((abc[0] - abc[2]).rem_euclid(face.k()), 0, "a ≡ c");
                assert_eq!(face.world(abc), p);
                let q = p.map(f64::from);
                let back = face.point(face.axes_of(q));
                assert!((0..3).all(|d| (back[d] - q[d]).abs() < 1e-12));
            }
        }
    }

    /** Seeded uniform samples in [lo, hi), so failures reproduce. */
    fn sampler(seed: u64) -> impl FnMut(f64, f64) -> f64 {
        let mut state = seed;
        move |lo, hi| {
            state = (state * 1103515245 + 12345) % 2147483648;
            lo + (hi - lo) * state as f64 / 2147483648.0
        }
    }

    #[test]
    fn faces_share_out_the_rays_each_to_the_face_of_the_nearest_axis() {
        let faces = faces();
        let mut random = sampler(4);
        for _ in 0..3000 {
            let ray = [random(-1.0, 1.0), random(-1.0, 1.0), random(-1.0, 1.0)];
            let towards = |f: &Face| {
                let a = f.axes_of(ray);
                a[2]
            };
            let nearest = (0..FACES).max_by(|&a, &b| towards(&faces[a]).total_cmp(&towards(&faces[b]))).unwrap();
            let owners: Vec<usize> = (0..FACES).filter(|&f| faces[f].holds(faces[f].axes_of(ray))).collect();
            assert_eq!(owners, [nearest], "{ray:?}");
            // Its tangents within the corners' polygon, and the planes'.
            let [x, y, z] = faces[nearest].axes_of(ray);
            let slant = faces[nearest].max_slant();
            assert!((x / z).abs() <= slant[0] + 1e-9 && (y / z).abs() <= slant[1] + 1e-9);
            assert!(faces[nearest].planes().iter().all(|n| n[0] * x + n[1] * y + n[2] * z >= -1e-9));
        }
    }
}
