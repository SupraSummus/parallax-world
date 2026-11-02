//! The parts of slice-selection.ts that baking needs, in the same units.

/** Spacing of a face's layers: see LAYER in view.ts. */
pub const LAYER: f64 = std::f64::consts::FRAC_1_SQRT_2;

/** M / 2^level, the bricks' lattice M·FCC (brickSpacing in slice-selection.ts). */
const BRICK: i32 = 8;

/** N = 2^level: see spacing in cells.ts. */
pub fn spacing(level: i32) -> i32 {
    1 << level
}

/** See ATOM_SHIFT in slice-selection.ts. */
const ATOM_SHIFT: i32 = 4;

/** See atomLevel: the level of the grid a slice of `level` reads. */
pub fn atom_level(level: i32) -> i32 {
    (level - ATOM_SHIFT).max(0)
}

// The rest is for slices as their grid sees them (in_grid in lib.rs): of
// level at most ATOM_SHIFT, made of single voxels, in the grid's units.

/** See siteLayers: the layers a slice's sites may be in, relative to its slab's first. */
pub fn site_layers(level: i32) -> (i32, i32) {
    debug_assert!(level <= ATOM_SHIFT);
    (0, (1 << level) - 1)
}

/** See thickness in slice-selection.ts: its voxels reach a layer past their sites'. */
pub fn thickness(level: i32) -> i32 {
    let (first, last) = site_layers(level);
    last - first + 2
}

/**
 * A chunk cell of a slice: in lattice units its sites' a and b reach
 * `columns` and `rows`, from (left, bottom); width and height in face
 * units. See chunkCell.
 */
pub struct Cell {
    pub left: i32,
    pub bottom: i32,
    pub columns: i32,
    pub rows: i32,
    pub width: f64,
    pub height: f64,
}

/** Chunk (i, j) of a slice of `level`: brick i of row j. See chunkCell. */
pub fn chunk_cell(level: i32, i: i32, j: i32) -> Cell {
    let m = BRICK << level;
    Cell {
        left: (2 * i + (j & 1) - 1) * m,
        bottom: j * m - m / 2,
        columns: 2 * m,
        rows: m,
        width: (2 * m) as f64 * LAYER,
        height: m as f64,
    }
}

/** See sliceCentre: halfway through what the slice's voxels span. */
pub fn slice_centre(level: i32, layer: i32) -> f64 {
    let (first, _) = site_layers(level);
    ((layer * (1 << level) + first - 1) as f64 + thickness(level) as f64 / 2.0) * LAYER
}

/**
 * How far (as a tangent) a ray from the eye, a site of N·FCC (N = 2^level)
 * whose cell holds the camera, may be from the camera's ray to the same
 * point, d layers ahead of the eye: 2N / d (see eyeError in
 * slice-selection.ts).
 */
pub fn eye_error(level: i32, d: f64) -> f64 {
    2.0 * spacing(level) as f64 / d
}

/**
 * The largest eye error baking allows for: the slicing keeps every voxel
 * at least 2N layers ahead of its eye (leakFreeDetail, and NEAR_DEPTH in
 * renderer.ts).
 */
pub const EYE_ERROR: f64 = 1.0;
