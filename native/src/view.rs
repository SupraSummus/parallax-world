//! The camera's axes (viewBasis in view.ts) and vectors in a face's coordinates.

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
