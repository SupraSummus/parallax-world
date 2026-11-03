//! The camera's view and the 12 faces around it (view.ts): each face has its
//! own stack of slices and draws the rays closer to its axis than to any
//! other's. See docs/architecture.md, section 4.

use bake::Lattice;

use crate::slicing::LAYER;

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
        let points: Vec<[f64; 2]> = if all_in_front {
            corners.iter().filter_map(|&c| self.project(c)).collect()
        } else {
            clip(&corners, |p| dot(p, f) - NEAR_PLANE).into_iter().filter_map(|p| self.project(p)).collect()
        };
        if points.is_empty() {
            return None;
        }
        let mut b = ScreenBox { min_x: f64::INFINITY, max_x: f64::NEG_INFINITY, min_y: f64::INFINITY, max_y: f64::NEG_INFINITY };
        for [x, y] in points {
            (b.min_x, b.max_x, b.min_y, b.max_y) = (b.min_x.min(x), b.max_x.max(x), b.min_y.min(y), b.max_y.max(y));
        }
        Some(b)
    }
}
