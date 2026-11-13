//! The camera's view and the 12 faces around it (view.ts): each face has its
//! own chunks and draws the rays closer to its axis than to any
//! other's. See docs/architecture.md, section 4.

use bake::slicing::LAYER;
use fcc::Lattice;

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

/** Focal length in pixels: the longer viewport side spans `fov` degrees. */
pub fn focal_length(fov: f64, width: f64, height: f64) -> f64 {
    width.max(height) / 2.0 / (fov.to_radians() / 2.0).tan()
}

/**
 * A face: its axes in world coordinates, and its lattice vectors R = √2 ·
 * right, U = up, F = √2 · forward (Face in view.ts).
 */
pub struct Face {
    pub axes: [Vec3; 3],
    pub lattice: Lattice,
}

/** One face per neighbour direction F, in the order of FACES in view.ts (fcc's lattices). */
pub fn faces() -> Vec<Face> {
    let scaled = |v: [i32; 3], s: f64| v.map(|c| c as f64 * s);
    fcc::lattices()
        .iter()
        .map(|&lattice @ Lattice { r, u, f }| Face { axes: [scaled(r, LAYER), scaled(u, 1.0), scaled(f, LAYER)], lattice })
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
        for s in [1.0, -1.0] {
            for t in [1.0, -1.0] {
                rays = clip(&rays, |p| p[2] - s * p[0] - t * std::f64::consts::SQRT_2 * p[1]);
            }
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
        Some([fold(0, true), fold(0, false), fold(1, true), fold(1, false)])
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

#[cfg(test)]
mod tests {
    use super::*;

    /** Whether a ray (x, y, z) in face coordinates is the face's: |x| + √2 · |y| ≤ z (owns in compositor.wgsl). */
    fn in_face(x: f64, y: f64, z: f64) -> bool {
        x.abs() + std::f64::consts::SQRT_2 * y.abs() <= z
    }

    /** Seeded uniform samples in [lo, hi), so failures reproduce (sampler in view.test.ts). */
    fn sampler(seed: u64) -> impl FnMut(f64, f64) -> f64 {
        let mut state = seed;
        move |lo, hi| {
            state = (state * 1103515245 + 12345) % 2147483648;
            lo + (hi - lo) * state as f64 / 2147483648.0
        }
    }

    fn cross(a: Vec3, b: Vec3) -> Vec3 {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
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
    fn view_basis_is_orthonormal_keeps_right_level_and_turns_right_and_up() {
        let mut random = sampler(1);
        for _ in 0..100 {
            let (yaw, pitch) = (random(-80.0, 80.0), random(-80.0, 80.0));
            let [right, up, forward] = view_basis(yaw, pitch);
            for (a, b) in [(right, up), (up, forward), (forward, right)] {
                assert!(dot(a, b).abs() < 1e-12);
            }
            for v in [right, up, forward] {
                assert!((dot(v, v) - 1.0).abs() < 1e-12);
            }
            assert_eq!(right[1], 0.0);
            assert_eq!(forward[0].signum(), yaw.signum());
            assert_eq!(forward[1].signum(), pitch.signum());
        }
    }

    // A mirrored face would draw its world mirrored.
    #[test]
    fn faces_are_rotations_one_looking_at_each_nearest_neighbour() {
        let faces = faces();
        for Face { axes: [right, up, forward], .. } in &faces {
            assert!((dot(cross(*right, *up), *forward) - 1.0).abs() < 1e-12);
        }
        let mut forwards: Vec<[i32; 3]> = faces.iter().map(|f| f.lattice.f).collect();
        forwards.sort();
        forwards.dedup();
        assert_eq!(forwards.len(), 12);
        // Nearest neighbours: two of x, y, z ±1, the third 0.
        assert!(forwards.iter().all(|f| f.iter().map(|c| c.abs()).sum::<i32>() == 2 && f.iter().all(|c| c.abs() <= 1)));
    }

    // Each ray belongs to the face whose axis is nearest; in_face must agree.
    #[test]
    fn faces_share_out_the_rays_each_to_the_face_of_the_nearest_axis() {
        let faces = faces();
        let mut random = sampler(4);
        for _ in 0..1000 {
            let ray = [random(-1.0, 1.0), random(-1.0, 1.0), random(-1.0, 1.0)];
            let nearest = (0..12).max_by(|&a, &b| dot(ray, faces[a].axes[2]).total_cmp(&dot(ray, faces[b].axes[2]))).unwrap();
            let owners: Vec<usize> = (0..12)
                .filter(|&f| {
                    let [x, y, z] = to_local(&faces[f].axes, ray);
                    in_face(x, y, z)
                })
                .collect();
            assert_eq!(owners, [nearest]);
        }
    }

    #[test]
    fn face_tangents_are_the_screen_within_the_face_and_none_of_faces_behind() {
        let faces = faces();
        let face = |f: [i32; 3]| faces.iter().find(|face| face.lattice.f == f).unwrap();
        // Looking along +x+z: the screen reaches past the face's rays at its sides.
        let [min_x, max_x, _, max_y] = view(45.0, 0.0, 400.0).face_tangents(face([1, 0, 1])).unwrap();
        assert!((min_x + 1.0).abs() < 1e-9 && (max_x - 1.0).abs() < 1e-9 && (max_y - LAYER).abs() < 1e-9);
        assert!(view(45.0, 0.0, 400.0).face_tangents(face([-1, 0, -1])).is_none());
    }

    #[test]
    fn face_tangents_cover_every_ray_on_screen_by_its_face() {
        let faces = faces();
        let mut random = sampler(3);
        for _ in 0..300 {
            let v = view(random(-180.0, 180.0), random(-90.0, 90.0), random(150.0, 1000.0));
            let world = screen_ray(&v, random(0.0, 800.0), random(0.0, 600.0));
            let face = faces
                .iter()
                .find(|f| {
                    let [x, y, z] = to_local(&f.axes, world);
                    in_face(x, y, z)
                })
                .unwrap();
            let [x, y, z] = to_local(&face.axes, world);
            let [min_x, max_x, min_y, max_y] = v.face_tangents(face).unwrap();
            assert!(x / z >= min_x - 1e-9 && x / z <= max_x + 1e-9);
            assert!(y / z >= min_y - 1e-9 && y / z <= max_y + 1e-9);
        }
    }
}
