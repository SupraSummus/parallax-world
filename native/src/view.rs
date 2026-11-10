//! The camera's view and the 12 faces around it (view.ts): each face has its
//! own chunks and draws the rays closer to its axis than to any
//! other's. See docs/architecture.md, section 4.

use bake::Lattice;
use bake::slicing::LAYER;

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

/** One face per neighbour direction F, in the order of FACES in view.ts (bake's lattices). */
pub fn faces() -> Vec<Face> {
    let scaled = |v: [i32; 3], s: f64| v.map(|c| c as f64 * s);
    bake::lattices()
        .iter()
        .map(|&lattice @ Lattice { r, u, f }| Face { axes: [scaled(r, LAYER), scaled(u, 1.0), scaled(f, LAYER)], lattice })
        .collect()
}

/** Whether a ray (x, y, z) in face coordinates is the face's: |x| + √2 · |y| ≤ z. */
pub fn in_face(x: f64, y: f64, z: f64) -> bool {
    x.abs() + std::f64::consts::SQRT_2 * y.abs() <= z
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

/** A rectangle on screen, pixels from the top-left. */
#[derive(Clone, Copy)]
pub struct ScreenBox {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl View {
    /** The camera's axes in a face's coordinates. */
    pub fn face_basis(&self, face: &Face) -> [Vec3; 3] {
        view_basis(self.yaw, self.pitch).map(|axis| to_local(&face.axes, axis))
    }

    pub fn on_screen(&self, b: &ScreenBox) -> bool {
        b.min_x < self.width && b.max_x > 0.0 && b.min_y < self.height && b.max_y > 0.0
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

    /** For a face: the screen box of the part in front of the camera of a camera-relative rectangle on the plane z = distance. */
    pub fn screen_boxes(&self, face: &Face) -> ScreenBoxes {
        ScreenBoxes { axes: self.face_basis(face), centre: [self.width / 2.0, self.height / 2.0], focal: self.focal }
    }
}

/** Points nearer the camera plane than this are cut off before projecting. */
const NEAR_PLANE: f64 = 1e-3;

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

/** screenBox in view.ts, for one view and face. */
pub struct ScreenBoxes {
    axes: [Vec3; 3],
    centre: [f64; 2],
    focal: f64,
}

impl ScreenBoxes {
    fn project(&self, p: Vec3) -> Option<[f64; 2]> {
        let [r, u, f] = self.axes;
        let z = dot(p, f);
        (z > 0.0).then(|| [self.centre[0] + dot(p, r) * self.focal / z, self.centre[1] - dot(p, u) * self.focal / z])
    }

    /** The box of [left, right] × [bottom, top] on the plane z = distance, camera-relative face coordinates. */
    pub fn of(&self, left: f64, right: f64, bottom: f64, top: f64, distance: f64) -> Option<ScreenBox> {
        let corners = [[left, bottom, distance], [right, bottom, distance], [right, top, distance], [left, top, distance]];
        let f = self.axes[2];
        let all_in_front = corners.iter().all(|&c| dot(c, f) > NEAR_PLANE);
        // No allocation unless clipped: a frame asks for hundreds of thousands.
        let mut b = ScreenBox { min_x: f64::INFINITY, max_x: f64::NEG_INFINITY, min_y: f64::INFINITY, max_y: f64::NEG_INFINITY };
        let mut add = |p: Vec3| {
            if let Some([x, y]) = self.project(p) {
                (b.min_x, b.max_x, b.min_y, b.max_y) = (b.min_x.min(x), b.max_x.max(x), b.min_y.min(y), b.max_y.max(y));
            }
        };
        if all_in_front {
            corners.into_iter().for_each(&mut add);
        } else {
            clip(&corners, |p| dot(p, f) - NEAR_PLANE).into_iter().for_each(&mut add);
        }
        (b.min_x <= b.max_x).then_some(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /** Not a face of the renderer: face coordinates are world coordinates. */
    fn identity() -> Face {
        Face { axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], lattice: Lattice { r: [1, 0, 0], u: [0, 1, 0], f: [0, 0, 1] } }
    }

    /** The world ray through pixel (sx, sy). */
    fn screen_ray(v: &View, sx: f64, sy: f64) -> Vec3 {
        let [r, u, f] = view_basis(v.yaw, v.pitch);
        let (a, b) = (sx - v.width / 2.0, v.height / 2.0 - sy);
        [0, 1, 2].map(|i| f[i] * v.focal + r[i] * a + u[i] * b)
    }

    /** Where point p, camera-relative in a face's coordinates, is on screen, if in front of the camera. */
    fn project(v: &View, face: &Face, p: Vec3) -> Option<[f64; 2]> {
        let world = [0, 1, 2].map(|i| face.axes[0][i] * p[0] + face.axes[1][i] * p[1] + face.axes[2][i] * p[2]);
        let [r, u, f] = view_basis(v.yaw, v.pitch).map(|axis| dot(world, axis));
        (f > 0.0).then(|| [v.width / 2.0 + r * v.focal / f, v.height / 2.0 - u * v.focal / f])
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

    fn visible(v: &View, left: f64, right: f64, bottom: f64, top: f64, distance: f64) -> bool {
        v.screen_boxes(&identity()).of(left, right, bottom, top, distance).is_some_and(|b| v.on_screen(&b))
    }

    #[test]
    fn screen_boxes_show_a_rectangle_around_the_view_direction_not_one_to_the_side_or_behind() {
        // The view direction meets z = 10 at about (1.76, -1.79).
        assert!(visible(&view(10.0, -10.0, 400.0), 1.0, 2.0, -2.0, -1.0, 10.0));
        assert!(!visible(&view(10.0, -10.0, 400.0), -30.0, -20.0, -2.0, -1.0, 10.0));
        assert!(!visible(&view(180.0, 0.0, 400.0), 1.0, 2.0, -2.0, -1.0, 10.0));
        // Looking along +x; the rectangle on z = 10 spans x from -50 to 50: from behind the camera into view.
        assert!(visible(&view(90.0, 0.0, 400.0), -50.0, 50.0, -1.0, 1.0, 10.0));
    }

    #[test]
    fn screen_boxes_bound_the_projected_corners_of_a_rectangle_in_front_of_the_camera() {
        let faces = faces();
        let mut random = sampler(5);
        let mut checked = 0;
        for _ in 0..300 {
            let v = view(random(-180.0, 180.0), random(-90.0, 90.0), 400.0);
            let face = &faces[random(0.0, 12.0) as usize];
            let (x, y, d) = (random(-20.0, 20.0), random(-20.0, 20.0), random(1.0, 30.0));
            let corners: Option<Vec<[f64; 2]>> =
                [[x, y], [x + 3.0, y], [x + 3.0, y + 2.0], [x, y + 2.0]].iter().map(|&[cx, cy]| project(&v, face, [cx, cy, d])).collect();
            let Some(corners) = corners else { continue };
            let b = v.screen_boxes(face).of(x, x + 3.0, y, y + 2.0, d).unwrap();
            let bound = |axis: usize, min: bool| corners.iter().map(|c| c[axis]).reduce(if min { f64::min } else { f64::max }).unwrap();
            for (got, want) in
                [(b.min_x, bound(0, true)), (b.max_x, bound(0, false)), (b.min_y, bound(1, true)), (b.max_y, bound(1, false))]
            {
                assert!((got - want).abs() < 1e-6, "{got} {want}");
            }
            checked += 1;
        }
        assert!(checked > 50);
    }
}
