//! Slice selection (slice-selection.ts), in the same units: which slices
//! of a face's layers the camera draws, the chunks they are split into,
//! and how far a chunk's voxels reach. Depths count layers, LAYER apart;
//! see docs/architecture.md, section 1. The baker reads slices as their
//! grid sees them (in_grid in lib.rs): of level at most ATOM_SHIFT, made of
//! single voxels, in the grid's units.

use crate::surface::Slice;

/** Spacing of a face's layers, and of its lattice along `right`: see LAYER in view.ts. */
pub const LAYER: f64 = std::f64::consts::FRAC_1_SQRT_2;

/** The coarsest level: slabs of 4096 layers. */
pub const MAX_LEVEL: i32 = 12;

/** Above this level a slab is 2^ATOM_SHIFT layers of cells, not of voxels. */
const ATOM_SHIFT: i32 = 4;
// A splat's depth in half layers of its grid, within ±2^ATOM_SHIFT, rides in a byte (Splat).
const _: () = assert!(1 << ATOM_SHIFT < 128);

/** The level of the cells a slice of `level` is made of, and so of the grid it reads (atomLevel). */
pub fn atom_level(level: i32) -> i32 {
    (level - ATOM_SHIFT).max(0)
}

/** Levels of a world's grids: one per atom level up to MAX_LEVEL's. */
pub const GRID_LEVELS: usize = (MAX_LEVEL - ATOM_SHIFT + 1) as usize;

/** N = 2^level, the spacing in layers of the sites of cells of `level`: see spacing in cells.ts. */
pub fn spacing(level: i32) -> i32 {
    1 << level
}

fn atom_spacing(level: i32) -> i32 {
    spacing(atom_level(level))
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

/** The layers the sites a slice of `level` stands for may be in, relative to the first layer of its slab (siteLayers). */
pub fn site_layers(level: i32) -> (i32, i32) {
    let (lo, hi) = owned_layers(atom_level(level));
    (lo, (1 << level) - atom_spacing(level) + hi)
}

/** A slice's thickness in layer spacings, 2^level + N: its voxels reach a layer past their sites' (thickness). */
pub fn thickness(level: i32) -> i32 {
    (1 << level) + atom_spacing(level)
}

/** Its near side, the near tips of its nearest voxels, in layers. */
fn near_side(level: i32, layer: i32) -> f64 {
    layer as f64 * (1 << level) as f64 - atom_spacing(level) as f64
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

/** The chunks (i, j) of a slice of `level` that meet [left, right) × [bottom, top) of its plane, in face units, row by row (chunkCells). */
pub fn chunk_cells(level: i32, left: f64, right: f64, bottom: f64, top: f64) -> Vec<(i32, i32)> {
    let m = (BRICK << level) as f64;
    let mut cells = Vec::new();
    let mut j = (bottom / m + 0.5).floor() as i32;
    while (j as f64 - 0.5) * m < top {
        let brick = |a: f64| (a / m - (j & 1) as f64 + 1.0) / 2.0;
        let mut i = brick(left / LAYER).floor() as i32;
        while (i as f64) < brick(right / LAYER) {
            cells.push((i, j));
            i += 1;
        }
        j += 1;
    }
    cells
}

/**
 * How far a face's rays are from its axis, as a tangent, at most: the
 * face's rays are |x| + √2 · |y| ≤ z (inFace in view.ts). Seen along them,
 * a splat d ahead of its slice's centre plane is drawn up to
 * MAX_SLANT · d off its site there (MAX_SLANT in slice-selection.ts).
 */
pub const MAX_SLANT: f64 = 1.0;

/** How far past its slice's centre plane a splat is per step of its depth byte, in world units (halfLayer in slice-selection.ts). */
pub fn half_layer(level: i32) -> f64 {
    atom_spacing(level) as f64 * LAYER / 2.0
}

/**
 * How far around its cell a chunk of a slice of `level` may draw voxels
 * from, in lattice units, along a and along b: within their reach
 * (Splat::reach), their sites less than half the slice's thickness off its
 * centre plane (cellMargin in slice-selection.ts).
 */
pub fn cell_margin(level: i32) -> (i32, i32) {
    let n = atom_spacing(level) as f64;
    let off = MAX_SLANT * (thickness(level) as f64 / n / 2.0) * LAYER;
    let columns = n * ((off + 1.5) / LAYER).ceil();
    let rows = n * (off * std::f64::consts::FRAC_1_SQRT_2 + 1.5).ceil();
    (columns as i32, rows as i32)
}

/** Viewing distance over slice thickness that keeps occlusion leaks under `max_leak` radians, rounded up to a quarter octave (leakFreeDetail in slice-selection.ts). */
pub fn leak_free_detail(max_leak: f64) -> f64 {
    let detail = (MAX_SLANT / max_leak - 0.5).max(1.0);
    2f64.powf((4.0 * detail.log2()).ceil() / 4.0)
}

/** z (face coordinates) of a slice's centre plane, halfway through what its voxels span: where its chunks are baked and drawn (sliceCentre). */
pub fn slice_centre(slice: &Slice) -> f64 {
    (near_side(slice.level, slice.layer) + thickness(slice.level) as f64 / 2.0) * LAYER
}

/**
 * Slices holding every site with c in [camera_z + near, camera_z + far),
 * each in exactly one, near to far, all in layers (selectSlices in
 * slice-selection.ts, which explains the cutoffs).
 */
pub fn select_slices(camera_z: f64, near: f64, far: f64, detail: f64) -> Vec<Slice> {
    let mut slices = Vec::new();
    if near >= far {
        return slices;
    }
    let far = camera_z + far;
    let mut from = (camera_z + near).ceil();
    for level in 0.. {
        let slab = (1u64 << level) as f64;
        let n = atom_spacing(level) as f64;
        let first = site_layers(level).0 as f64;
        let (mut until, mut grows) = (f64::INFINITY, false);
        if level < MAX_LEVEL {
            let next = level + 1;
            grows = atom_level(next) > atom_level(level);
            let at_least = (camera_z - near_side(next, 0) + thickness(next) as f64 * detail).max(from + if grows { 3.0 * n } else { 0.0 });
            until = (at_least / (2.0 * slab)).ceil() * 2.0 * slab;
            if until + site_layers(next).0 as f64 >= far {
                until = f64::INFINITY;
            }
        }
        let mut layer = (from / slab).ceil();
        loop {
            let more = if until == f64::INFINITY {
                layer * slab + first < far
            } else if grows {
                layer * slab <= until
            } else {
                layer * slab < until
            };
            if !more {
                break;
            }
            let cutoff = if grows && (layer + 1.0) * slab + 4.0 * n + 2.0 > until { until } else { f64::INFINITY };
            slices.push(Slice { level, layer: layer as i32, cutoff });
            layer += 1.0;
        }
        if until == f64::INFINITY {
            return slices;
        }
        from = until;
    }
    unreachable!("the last level reaches any distance")
}
