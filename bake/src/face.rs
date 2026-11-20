//! A face's measures, seen along its axis: depths count layers, LAYER
//! apart (docs/architecture.md, section 1).

/** Spacing of a face's layers, and of its lattice along `right`. */
pub const LAYER: f64 = std::f64::consts::FRAC_1_SQRT_2;

/** N = 2^level, the spacing in layers of the sites of cells of `level`. */
pub fn spacing(level: i32) -> i32 {
    1 << level
}

/**
 * How far a face's rays are from its axis, as a tangent, at most: the
 * face's rays are |x| + √2 · |y| ≤ z (in_face in native's view.rs). Seen
 * along them, a splat d ahead of its chunk's plane is drawn up to
 * MAX_SLANT · d off its site there.
 */
pub const MAX_SLANT: f64 = 1.0;

/** How far past its chunk's plane a splat is per half layer of its depth, in world units, its voxels of level `atoms`. */
pub fn half_layer(atoms: i32) -> f64 {
    spacing(atoms) as f64 * LAYER / 2.0
}
