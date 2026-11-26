//! The camera's view: its axes in the world, its focal length; and the 18
//! faces around it, along the cube's axes and edges, each of which draws
//! the rays closer to its axis than to any other's (bake's face.rs). See
//! docs/architecture.md, section 4.

pub type Vec3 = [f64; 3];

/** Right, up, forward in world coordinates; yaw and pitch in degrees. */
pub fn view_basis(yaw: f64, pitch: f64) -> [Vec3; 3] {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    [[y.cos(), 0.0, -y.sin()], [-y.sin() * p.sin(), p.cos(), -y.cos() * p.sin()], [y.sin() * p.cos(), p.sin(), y.cos() * p.cos()]]
}

pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/** World vector v in the coordinates of `axes`. */
pub fn to_local(axes: &[Vec3; 3], v: Vec3) -> Vec3 {
    [dot(v, axes[0]), dot(v, axes[1]), dot(v, axes[2])]
}

/** A face: its axes in world coordinates, right, up and forward, and its lattice vectors (bake's Face). */
pub struct Face {
    pub axes: [Vec3; 3],
    pub face: bake::face::Face,
}

/** One face per direction, in the order of bake's faces. */
pub fn faces() -> Vec<Face> {
    bake::face::faces()
        .into_iter()
        .map(|face| {
            let l = face.layer();
            let scaled = |v: [i32; 3], s: f64| v.map(|c| f64::from(c) * s);
            Face { axes: [scaled(face.r, l), scaled(face.u, 1.0), scaled(face.f, l)], face }
        })
        .collect()
}

/** A view: orientation in degrees, viewport in pixels and focal length. */
#[derive(Clone, Copy, PartialEq)]
pub struct View {
    pub yaw: f64,
    pub pitch: f64,
    pub width: f64,
    pub height: f64,
    pub focal: f64,
}

impl View {
    /** The camera's axes in a face's coordinates. */
    pub fn face_basis(&self, face: &Face) -> [Vec3; 3] {
        view_basis(self.yaw, self.pitch).map(|axis| to_local(&face.axes, axis))
    }

    /** Bounds of x / z and y / z of the face's rays the screen shows ([min x, max x, min y, max y]), or None if it shows none. */
    pub fn face_tangents(&self, face: &Face) -> Option<[f64; 4]> {
        let [r, u, f] = self.face_basis(face);
        let ray = |sx: f64, sy: f64| {
            let (a, b) = (sx - self.width / 2.0, self.height / 2.0 - sy);
            [0, 1, 2].map(|i| f[i] * self.focal + r[i] * a + u[i] * b)
        };
        let mut rays = vec![ray(0.0, 0.0), ray(self.width, 0.0), ray(self.width, self.height), ray(0.0, self.height)];
        for n in face.face.planes() {
            rays = clip(&rays, |p| dot(n, p));
        }
        let tangents: Vec<[f64; 2]> = rays.iter().map(|r| [r[0] / r[2], r[1] / r[2]]).collect();
        // A screen meeting the face only along an edge shows nothing of it.
        let area: f64 = (0..tangents.len())
            .map(|i| {
                let (p, q) = (tangents[i], tangents[(i + 1) % tangents.len()]);
                p[0] * q[1] - q[0] * p[1]
            })
            .sum();
        if area.abs().is_nan() || area.abs() <= 1e-9 {
            return None;
        }
        let fold = |axis: usize, min: bool| {
            tangents
                .iter()
                .map(|t| t[axis])
                .fold(if min { f64::INFINITY } else { f64::NEG_INFINITY }, if min { f64::min } else { f64::max })
        };
        // Within its rays' tangents, which clipping reaches but to rounding.
        let [x, y] = face.face.max_slant();
        Some([fold(0, true).max(-x), fold(0, false).min(x), fold(1, true).max(-y), fold(1, false).min(y)])
    }
}

/** Clips a convex polygon to distance(p) ≥ 0 (Sutherland–Hodgman), for a linear `distance`. */
fn clip(polygon: &[Vec3], distance: impl Fn(Vec3) -> f64) -> Vec<Vec3> {
    let mut out = Vec::new();
    for (i, &p) in polygon.iter().enumerate() {
        let q = polygon[(i + 1) % polygon.len()];
        let (dp, dq) = (distance(p), distance(q));
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp < 0.0) != (dq < 0.0) {
            let t = dp / (dp - dq);
            out.push([0, 1, 2].map(|k| p[k] + (q[k] - p[k]) * t));
        }
    }
    out
}

/** Focal length in pixels: the longer viewport side spans `fov` degrees. */
pub fn focal_length(fov: f64, width: f64, height: f64) -> f64 {
    width.max(height) / 2.0 / (fov.to_radians() / 2.0).tan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_basis_is_orthonormal_keeps_right_level_and_turns_right_and_up() {
        for (yaw, pitch) in [(-80.0, 30.0), (10.0, -45.0), (60.0, 75.0), (-5.0, -5.0)] {
            let [right, up, forward] = view_basis(yaw, pitch);
            for (a, b) in [(right, up), (up, forward), (forward, right)] {
                assert!(dot(a, b).abs() < 1e-12);
            }
            for v in [right, up, forward] {
                assert!((dot(v, v) - 1.0).abs() < 1e-12);
            }
            assert_eq!(right[1], 0.0);
            assert_eq!(forward[0].signum(), f64::signum(yaw));
            assert_eq!(forward[1].signum(), f64::signum(pitch));
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

    fn view(yaw: f64, pitch: f64, focal: f64) -> View {
        View { yaw, pitch, width: 800.0, height: 600.0, focal }
    }

    /** The world ray through pixel (sx, sy). */
    fn screen_ray(v: &View, sx: f64, sy: f64) -> Vec3 {
        let [r, u, f] = view_basis(v.yaw, v.pitch);
        let (a, b) = (sx - v.width / 2.0, v.height / 2.0 - sy);
        [0, 1, 2].map(|i| f[i] * v.focal + r[i] * a + u[i] * b)
    }

    #[test]
    fn face_tangents_are_the_screen_within_the_face_and_none_of_faces_behind() {
        let faces = faces();
        let face = |f: [i32; 3]| faces.iter().find(|face| face.face.f == f).unwrap();
        // Looking along +x+z: the screen reaches past the face's rays at its sides.
        let [min_x, max_x, _, max_y] = view(45.0, 0.0, 400.0).face_tangents(face([1, 0, 1])).unwrap();
        let t = bake::face::TAN;
        assert!((min_x + t).abs() < 1e-9 && (max_x - t).abs() < 1e-9 && (max_y - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        assert!(view(45.0, 0.0, 400.0).face_tangents(face([-1, 0, -1])).is_none());
    }

    #[test]
    fn face_tangents_cover_every_ray_on_screen_by_its_face() {
        let faces = faces();
        let mut random = sampler(3);
        for _ in 0..300 {
            let v = view(random(-180.0, 180.0), random(-90.0, 90.0), random(150.0, 1000.0));
            let world = screen_ray(&v, random(0.0, 800.0), random(0.0, 600.0));
            let face = faces.iter().find(|f| f.face.holds(to_local(&f.axes, world))).unwrap();
            let [x, y, z] = to_local(&face.axes, world);
            let [min_x, max_x, min_y, max_y] = v.face_tangents(face).unwrap();
            assert!(x / z >= min_x - 1e-9 && x / z <= max_x + 1e-9);
            assert!(y / z >= min_y - 1e-9 && y / z <= max_y + 1e-9);
        }
    }
}
