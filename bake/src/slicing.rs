//! Slices' shapes (slice-selection.ts, which selects them for the web), in
//! the same units: the chunks they are split into, and how far a chunk's
//! voxels reach. Depths count layers, LAYER apart; see
//! docs/architecture.md, section 1. The baker reads slices as their grid
//! sees them (in_grid in lib.rs): of level at most ATOM_SHIFT, made of
//! single voxels, in the grid's units. Functions of a slice's shape take
//! its level and its atoms' (see Slice).

use crate::surface::Slice;

/** Spacing of a face's layers, and of its lattice along `right`: see LAYER in view.ts. */
pub const LAYER: f64 = std::f64::consts::FRAC_1_SQRT_2;

/** The coarsest level: slabs of 4096 layers. */
pub const MAX_LEVEL: i32 = 12;

/** The largest atom shift: slabs of at most 2^ATOM_SHIFT layers of cells (ATOM_SHIFT in slice-selection.ts). */
const ATOM_SHIFT: i32 = 4;
// A splat's depth in half layers of its grid, within ±2^ATOM_SHIFT, rides in a byte (Splat).
const _: () = assert!(1 << ATOM_SHIFT < 128);

/** Levels of a world's grids: one per atom level up to MAX_LEVEL's at the largest shift. */
pub const GRID_LEVELS: usize = (MAX_LEVEL - ATOM_SHIFT + 1) as usize;

/** How far around the camera each grid holds the world, in its units, and slices read it (GRID_REACH in slice-selection.ts). */
pub const GRID_REACH: f64 = 400.0;

/** N = 2^level, the spacing in layers of the sites of cells of `level`: see spacing in cells.ts. */
pub fn spacing(level: i32) -> i32 {
    1 << level
}

fn atom_spacing(atoms: i32) -> i32 {
    spacing(atoms)
}

/** The layers c − c0 of the sites a cell of `level` holds, c0 its own site's layer (ownedLayers in cells.ts). */
fn owned_layers(level: i32) -> (i32, i32) {
    let n = spacing(level);
    match level {
        0 => (0, 0),
        1 => (-2, 0),
        _ => (-n, n - 1),
    }
}

/** The layers the sites a slice of `level` and `atoms` stands for may be in, relative to the first layer of its slab (siteLayers). */
pub fn site_layers(level: i32, atoms: i32) -> (i32, i32) {
    let (lo, hi) = owned_layers(atoms);
    (lo, (1 << level) - atom_spacing(atoms) + hi)
}

/** A slice's thickness in layer spacings, 2^level + N: its voxels reach a layer past their sites' (thickness). */
pub fn thickness(level: i32, atoms: i32) -> i32 {
    (1 << level) + atom_spacing(atoms)
}

/** Its near side, the near tips of its nearest voxels, in layers. */
fn near_side(level: i32, atoms: i32, layer: i32) -> f64 {
    layer as f64 * (1 << level) as f64 - atom_spacing(atoms) as f64
}

/** M / 2^level, the bricks' lattice M·FCC (brickSpacing in slice-selection.ts). */
const BRICK: i32 = 8;

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

/** Chunk (i, j) of a slice of `level`: brick i of row j, rows offset by half a brick. See chunkCell. */
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

/**
 * How far a face's rays are from its axis, as a tangent, at most: the
 * face's rays are |x| + √2 · |y| ≤ z (inFace in view.ts). Seen along them,
 * a splat d ahead of its slice's centre plane is drawn up to
 * MAX_SLANT · d off its site there (MAX_SLANT in slice-selection.ts).
 */
pub const MAX_SLANT: f64 = 1.0;

/** How far past its slice's centre plane a splat is per step of its depth byte, in world units (halfLayer in slice-selection.ts). */
pub fn half_layer(atoms: i32) -> f64 {
    atom_spacing(atoms) as f64 * LAYER / 2.0
}

/**
 * How far around its cell a chunk of a slice of `level` may draw voxels
 * from, in lattice units, along a and along b: within their reach
 * (Splat::reach), their sites less than half the slice's thickness off its
 * centre plane (cellMargin in slice-selection.ts).
 */
pub fn cell_margin(level: i32, atoms: i32) -> (i32, i32) {
    let n = atom_spacing(atoms) as f64;
    let off = MAX_SLANT * (thickness(level, atoms) as f64 / n / 2.0) * LAYER;
    let columns = n * ((off + 1.5) / LAYER).ceil();
    let rows = n * (off * std::f64::consts::FRAC_1_SQRT_2 + 1.5).ceil();
    (columns as i32, rows as i32)
}

/** z (face coordinates) of a slice's centre plane, halfway through what its voxels span: where its chunks are baked and drawn (sliceCentre). */
pub fn slice_centre(slice: &Slice) -> f64 {
    (near_side(slice.level, slice.atoms, slice.layer) + thickness(slice.level, slice.atoms) as f64 / 2.0) * LAYER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slice_centre_is_halfway_through_what_its_voxels_span() {
        let centre = |level, atoms, layer| slice_centre(&Slice { level, atoms, layer, cutoff: f64::INFINITY }) / LAYER;
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} {b}");
        // Level 3, slabs of 8 voxel layers: sites in layers 40 to 47, voxels reaching from 39 to 48.
        close(centre(3, 0, 5), 43.5);
        // Level 4, sites in layers 16 to 31, voxels reaching from 15 to 32.
        close(centre(4, 0, 1), 23.5);
        // Level 5, slabs of 16 layers of voxels of 2·FCC: their sites in layers 32 to 62, reaching from 30 to 64.
        close(centre(5, 1, 1), 47.0);
        // Level 6, voxels of 4·FCC in layers 64 to 124, reaching from 60 to 128.
        close(centre(6, 2, 1), 94.0);
        // Level 4 at atom shift 2: 4 layers of voxels of 4·FCC, their sites in layers 16 to 28, reaching from 12 to 32.
        close(centre(4, 2, 1), 22.0);
    }
}
