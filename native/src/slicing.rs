//! Slice selection (slice-selection.ts): which slices of a face's layers the
//! camera draws, the chunks they are split into and the eyes they are baked
//! from. Depths count layers, LAYER apart; see docs/architecture.md, section 1.

use bake::Slice;

use crate::view::Vec3;

/** Spacing of a face's layers, and of its lattice along `right`. */
pub const LAYER: f64 = std::f64::consts::FRAC_1_SQRT_2;

/** The coarsest level: slabs of 4096 layers. */
pub const MAX_LEVEL: i32 = 12;

/** Above this level a slab is 2^ATOM_SHIFT layers of cells, not of voxels. */
const ATOM_SHIFT: i32 = 4;

/** The level of the cells a slice of `level` is made of, and so of the grid it reads. */
pub fn atom_level(level: i32) -> i32 {
    (level - ATOM_SHIFT).max(0)
}

/** Levels of a world's grids: one per atom level up to MAX_LEVEL's. */
pub const GRID_LEVELS: usize = (MAX_LEVEL - ATOM_SHIFT + 1) as usize;

/** N, the spacing in layers of the sites of cells of `level`. */
fn spacing(level: i32) -> i32 {
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

/** The layers the sites a slice of `level` stands for may be in, relative to the first layer of its slab. */
pub fn site_layers(level: i32) -> (i32, i32) {
    let (lo, hi) = owned_layers(atom_level(level));
    (lo, (1 << level) - atom_spacing(level) + hi)
}

/** A slice's thickness in layer spacings: 2^level + N. */
fn thickness(level: i32) -> i32 {
    (1 << level) + atom_spacing(level)
}

/** Its near side, the near tips of its nearest voxels, in layers. */
fn near_side(level: i32, layer: i32) -> f64 {
    layer as f64 * (1 << level) as f64 - atom_spacing(level) as f64
}

/** M: the spacing of the lattice whose columns of cells a slice of `level` is split along. */
fn brick_spacing(level: i32) -> f64 {
    (8 << level) as f64
}

/** A part of a slice's plane, in face coordinates: [x, x + width) × [y, y + height). */
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/** Chunk (i, j) of a slice of `level`: brick i of row j, rows offset by half a brick. */
pub fn chunk_cell(level: i32, i: i32, j: i32) -> Rect {
    let m = brick_spacing(level);
    Rect { x: (2 * i + (j & 1) - 1) as f64 * m * LAYER, y: (j as f64 - 0.5) * m, width: 2.0 * m * LAYER, height: m }
}

/** The chunks (i, j) of a slice of `level` that meet [left, right) × [bottom, top) of its plane, row by row. */
pub fn chunk_cells(level: i32, left: f64, right: f64, bottom: f64, top: f64) -> Vec<(i32, i32)> {
    let m = brick_spacing(level);
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

/** The site of N·FCC whose cell holds the point (a, b, c), in lattice coordinates (owner in cells.ts). */
fn owner(n: f64, p: Vec3) -> [i32; 3] {
    let mut best = [0.0; 3];
    let mut best_d = f64::INFINITY;
    for parity in [0.0, 1.0] {
        let around = |x: f64| {
            let t = ((x - parity * n) / (2.0 * n)).floor();
            [n * (2.0 * t + parity), n * (2.0 * t + 2.0 + parity)]
        };
        let [xa, xb, xc] = p.map(around);
        for a in xa {
            for b in xb {
                for c in xc {
                    let d = (a - p[0]).powi(2) + 2.0 * (b - p[1]).powi(2) + (c - p[2]).powi(2);
                    if d < best_d || (d == best_d && (c, b, a) > (best[2], best[1], best[0])) {
                        best = [a, b, c];
                        best_d = d;
                    }
                }
            }
        }
    }
    best.map(|v| v as i32)
}

/** The eye of a slice of `level` for a camera at `camera` (face coordinates): the site of the level's lattice whose cell holds it. */
pub fn eye_of(level: i32, camera: Vec3) -> [i32; 3] {
    owner(spacing(level) as f64, [camera[0] / LAYER, camera[1], camera[2] / LAYER])
}

/** How far past the face's rays a ray from the eye may be, plus how far gap filling may grow faces. */
const REACH: f64 = 2.0;

/** How far around its cell a chunk of a slice of `level` may draw voxels from, in lattice units: along a, along b. */
pub fn cell_margin(level: i32) -> (i32, i32) {
    let n = atom_spacing(level) as f64;
    let half = (thickness(level) as f64 / n / 2.0) * LAYER;
    let columns = n * (((1.0 + REACH) * half + 1.5) / LAYER).ceil();
    let rows = n * ((LAYER + REACH) * half + 1.5).ceil();
    (columns as i32, rows as i32)
}

/** The largest N / thickness of any level. */
fn eye_ratio() -> f64 {
    (0..=MAX_LEVEL).map(|level| spacing(level) as f64 / thickness(level) as f64).fold(0.0, f64::max)
}

/** Viewing distance over the largest slice thickness that keeps occlusion leaks under `max_leak` radians, rounded up to a quarter octave. */
pub fn leak_free_detail(max_leak: f64) -> f64 {
    let r = eye_ratio();
    let detail = (3.0 * r).max((r + (r * r + 8.0 * r / max_leak).sqrt()) / 2.0);
    2f64.powf((4.0 * detail.log2()).ceil() / 4.0)
}

/** z (face coordinates) of a slice's centre plane, where its chunks are baked and drawn. */
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
