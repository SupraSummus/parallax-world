//! Slice selection (slice-selection.ts), in the same units: which slices
//! of a face's layers the camera draws, the chunks they are split into,
//! and how far a chunk's voxels reach. Depths count layers, LAYER apart;
//! see docs/architecture.md, section 1. The baker reads slices as their
//! grid sees them (in_grid in lib.rs): of level at most ATOM_SHIFT, made of
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

/** The level of the cells a slice of `level` is made of at atom shift `shift`, and so of the grid it reads (atomLevel). */
pub fn atom_level(level: i32, shift: i32) -> i32 {
    (level - shift).clamp(0, GRID_LEVELS as i32 - 1)
}

/** The atom shift for slices of `detail`: the finest atoms whose slices read no grid past GRID_REACH (atomShift, which explains it). */
pub fn atom_shift(detail: f64) -> i32 {
    let reach = |s: i32| thickness(s + 1, 1) as f64 * detail + 2.0 + 2.0 * (1 << s) as f64 + 3.0 + 1.0 + 1.0;
    let mut shift = ATOM_SHIFT;
    while shift > 0 && reach(shift) > GRID_REACH {
        shift -= 1;
    }
    shift
}

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
    let shift = atom_shift(detail);
    let mut from = (camera_z + near).ceil();
    for level in 0.. {
        let slab = (1u64 << level) as f64;
        let atoms = atom_level(level, shift);
        let n = atom_spacing(atoms) as f64;
        let first = site_layers(level, atoms).0 as f64;
        let (mut until, mut grows) = (f64::INFINITY, false);
        if level < MAX_LEVEL {
            let (next, next_atoms) = (level + 1, atom_level(level + 1, shift));
            grows = next_atoms > atoms;
            let at_least = (camera_z - near_side(next, next_atoms, 0) + thickness(next, next_atoms) as f64 * detail)
                .max(from + if grows { 3.0 * n } else { 0.0 });
            until = (at_least / (2.0 * slab)).ceil() * 2.0 * slab;
            if until + site_layers(next, next_atoms).0 as f64 >= far {
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
            slices.push(Slice { level, atoms, layer: layer as i32, cutoff });
            layer += 1.0;
        }
        if until == f64::INFINITY {
            return slices;
        }
        from = until;
    }
    unreachable!("the last level reaches any distance")
}

/** A run of consecutive slices of one slicing and the slices of another it replaces, by their indices (see match_tilings). */
pub struct Segment {
    pub wanted: std::ops::Range<usize>,
    pub previous: std::ops::Range<usize>,
}

/**
 * Pairs up two slicings of a face (near to far, as select_slices returns
 * them): segments of consecutive `wanted` slices, each with the `previous`
 * slices it replaces. Slices in both are segments of their own, paired
 * with themselves; the ones between two of them, or before the first, are
 * paired with the previous slices there, which hold the same sites but for
 * a few at the ends. Wanted slices past the last shared one get none
 * (matchTilings in slice-selection.ts).
 */
pub fn match_tilings(wanted: &[Slice], previous: &[Slice]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut start = 0;
    // Just past the last slice found in both, by its index in `previous`.
    let mut shared = 0;
    for (w, slice) in wanted.iter().enumerate() {
        // A slice of both lies past the last one found.
        let Some(k) = previous[shared..].iter().position(|p| p == slice).map(|k| k + shared) else { continue };
        if start < w {
            segments.push(Segment { wanted: start..w, previous: shared..k });
        }
        segments.push(Segment { wanted: w..w + 1, previous: k..k + 1 });
        start = w + 1;
        shared = k + 1;
    }
    if start < wanted.len() {
        segments.push(Segment { wanted: start..wanted.len(), previous: shared..shared });
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::owner;
    use std::collections::HashSet;

    /** The panel's range of detail (MIN_DETAIL to MAX_DETAIL in renderer.ts), and its farthest render distance. */
    const MIN_DETAIL: f64 = 1.0;
    const MAX_DETAIL: f64 = 64.0;
    const MAX_RENDER_DISTANCE: f64 = 40000.0;

    fn name(s: &Slice) -> String {
        if s.cutoff.is_finite() { format!("{}:{}|{}", s.level, s.layer, s.cutoff) } else { format!("{}:{}", s.level, s.layer) }
    }

    fn names(slices: &[Slice]) -> Vec<String> {
        slices.iter().map(name).collect()
    }

    /** Whether a slice holds site (a, b, c) of any grid: its atom (classify in surface.rs). */
    fn holds(s: &Slice, site: [i32; 3]) -> bool {
        let n = spacing(s.atoms);
        owner(n, site)[2].div_euclid(1 << s.level) == s.layer && (s.cutoff == f64::INFINITY || (owner(2 * n, site)[2] as f64) < s.cutoff)
    }

    #[test]
    fn slices_as_docs_architecture_md_has_them_for_camera_z_0() {
        // Section 1, detail 1: level:layer, |cutoff.
        assert_eq!(
            names(&select_slices(0.0, 1.0, 2000.0, 1.0)).join(" "),
            "0:1 0:2 0:3 1:2 1:3 2:2 2:3 3:2 3:3 4:2 4:3|64 4:4|64 5:2 5:3|128 5:4|128 6:2 6:3|256 6:4|256 7:2 7:3|512 7:4|512 8:2 8:3|1024 8:4|1024 9:2 9:3|2048 9:4|2048 10:2"
        );
    }

    #[test]
    fn every_site_ahead_in_exactly_one_slice_near_to_far_none_too_thick() {
        for detail in [1.0, 2.0, 4.0, 8.0, 16.0] {
            for i in 0..21 {
                let camera_z = -100.0 + i as f64 * 9.37;
                let slices = select_slices(camera_z, 1.0, 120.0, detail);
                let near = (camera_z + 1.0).ceil() as i32;
                // Every site from the first layer ahead to the far end is in
                // exactly one slice; none nearer (a slice close to the camera
                // would be blown up over the screen).
                for c in near - 3..(camera_z + 120.0).floor() as i32 {
                    for a in (-6 + (c & 1)..=6).step_by(2) {
                        for b in (-6 + (c & 1)..=6).step_by(2) {
                            let count = slices.iter().filter(|s| holds(s, [a, b, c])).count();
                            assert_eq!(count, usize::from(c >= near), "detail {detail} camera {camera_z} site {a},{b},{c}");
                        }
                    }
                }
                for w in slices.windows(2) {
                    // Level by level, layer by layer. (Where a level ends, its
                    // last slices hold sites of cells one level up before the
                    // next level's first layer, so they come first, though
                    // their centre planes may be a little farther.)
                    let (p, q) = (&w[0], &w[1]);
                    assert!(q.level > p.level || (q.level == p.level && q.layer == p.layer + 1), "{} {}", name(p), name(q));
                    assert!(slice_centre(q) > slice_centre(p) - 2.0 * LAYER);
                }
                for s in &slices {
                    // Past level 0, never thicker than (distance to the near side) / detail.
                    let ahead = near_side(s.level, s.atoms, s.layer) - camera_z;
                    assert!(s.level == 0 || thickness(s.level, s.atoms) as f64 <= ahead / detail, "{}", name(s));
                }
                // Logarithmic, not linear, in the view distance.
                assert!(slices.len() as f64 <= 12.0 * detail + 30.0);
            }
        }
    }

    #[test]
    fn the_same_slices_from_nearby_cameras() {
        // Moving one layer forward keeps all but the slices near where levels change.
        let before = names(&select_slices(0.0, 1.0, 400.0, 16.0));
        let after = names(&select_slices(1.0, 1.0, 400.0, 16.0));
        let before: HashSet<_> = before.iter().collect();
        assert!(after.iter().filter(|k| before.contains(k)).count() as f64 > after.len() as f64 * 0.9);
    }

    /*
     * At every detail the panel offers and wherever the camera is, no
     * slice reads its grid farther than GRID_REACH of it, in the grid's
     * units: the sites it holds, before its slab's end or less than 3N past
     * its cutoff (select_slices), their neighbours and voxels 2N more.
     * A site c layers ahead is at most c units away (atom_shift).
     */
    #[test]
    fn slices_keep_within_grid_reach_of_their_grids() {
        let octaves = 4 * (MAX_DETAIL / MIN_DETAIL).log2() as i32;
        for k in 0..=octaves {
            let detail = MIN_DETAIL * 2f64.powf(k as f64 / 4.0);
            let mut camera_z = -60000.0;
            while camera_z < 60000.0 {
                for s in select_slices(camera_z, 4.0, MAX_RENDER_DISTANCE / LAYER, detail) {
                    let n = atom_spacing(s.atoms) as f64;
                    let last = ((s.layer * (1 << s.level) + site_layers(s.level, s.atoms).1) as f64).min(s.cutoff + 3.0 * n);
                    assert!((last + 2.0 * n - camera_z) / n <= GRID_REACH, "detail {detail} camera {camera_z} slice {}", name(&s));
                }
                camera_z += 2997.3;
            }
        }
    }

    fn slice(level: i32, layer: i32, cutoff: f64) -> Slice {
        Slice { level, atoms: 0, layer, cutoff }
    }

    /** The segments' slices by name. */
    fn matched(wanted: &[Slice], previous: &[Slice]) -> Vec<(Vec<String>, Vec<String>)> {
        match_tilings(wanted, previous).into_iter().map(|g| (names(&wanted[g.wanted]), names(&previous[g.previous]))).collect()
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tilings_pair_slices_in_both_with_themselves_and_those_between_with_what_is_between() {
        let inf = f64::INFINITY;
        let previous = [slice(0, 1, inf), slice(0, 2, 6.0), slice(0, 3, 6.0), slice(1, 3, inf), slice(1, 4, inf)];
        let wanted = [slice(0, 1, inf), slice(0, 2, inf), slice(0, 3, 8.0), slice(0, 4, 8.0), slice(1, 4, inf)];
        assert_eq!(
            matched(&wanted, &previous),
            [
                (strings(&["0:1"]), strings(&["0:1"])),
                (strings(&["0:2", "0:3|8", "0:4|8"]), strings(&["0:2|6", "0:3|6", "1:3"])),
                (strings(&["1:4"]), strings(&["1:4"])),
            ]
        );
    }

    #[test]
    fn tilings_let_the_first_segment_stand_on_nearer_slices_and_pair_nothing_past_the_last_shared_one() {
        let inf = f64::INFINITY;
        let wanted = [slice(0, 2, inf), slice(0, 3, inf), slice(1, 2, inf)];
        let previous = [slice(0, 1, inf), slice(0, 2, 6.0), slice(0, 3, inf)];
        assert_eq!(
            matched(&wanted, &previous),
            [(strings(&["0:2"]), strings(&["0:1", "0:2|6"])), (strings(&["0:3"]), strings(&["0:3"])), (strings(&["1:2"]), vec![]),]
        );
    }

    #[test]
    fn tilings_cover_every_wanted_slice_once_in_order_and_previous_ones_at_most_once() {
        for camera_z in (-40..40).step_by(3) {
            for previous_z in [camera_z - 5, camera_z, camera_z + 7] {
                let wanted = select_slices(camera_z as f64, 1.0, 300.0, 2.0);
                let previous = select_slices(previous_z as f64, 1.0, 300.0, 2.0);
                let segments = match_tilings(&wanted, &previous);
                let covered: Vec<usize> = segments.iter().flat_map(|g| g.wanted.clone()).collect();
                assert_eq!(covered, (0..wanted.len()).collect::<Vec<_>>());
                let replaced: Vec<usize> = segments.iter().flat_map(|g| g.previous.clone()).collect();
                assert!(replaced.windows(2).all(|w| w[0] < w[1]));
            }
        }
    }

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

    #[test]
    fn chunk_cells_tile_the_plane_with_bricks_each_around_a_site_of_m_fcc() {
        for level in [0, 2] {
            let m = BRICK << level;
            // Sites of M·FCC seen along the axis: a and b multiples of M, a / M and b / M of one parity.
            for a in (-4 * m..=4 * m).step_by(m as usize) {
                for b in (-4 * m..=4 * m).step_by(m as usize) {
                    if ((a - b) / m) & 1 != 0 {
                        continue;
                    }
                    let (x, y) = (a as f64 * LAYER, b as f64);
                    let inside = chunk_cells(level, x, x + 1e-9, y, y + 1e-9);
                    assert_eq!(inside.len(), 1);
                    // In the middle of its brick.
                    let cell = chunk_cell(level, inside[0].0, inside[0].1);
                    assert!((cell.left as f64 * LAYER + cell.width / 2.0 - x).abs() < 1e-9);
                    assert!((cell.bottom as f64 + cell.height / 2.0 - y).abs() < 1e-9);
                }
            }
            // Points of the plane are each in exactly one brick.
            for k in 0..200 {
                let (x, y) = ((k as f64 * 12.9898).sin() * 300.0, (k as f64 * 78.233).cos() * 300.0);
                let holding = chunk_cells(level, x - 50.0, x + 50.0, y - 50.0, y + 50.0)
                    .into_iter()
                    .map(|(i, j)| chunk_cell(level, i, j))
                    .filter(|c| {
                        let left = c.left as f64 * LAYER;
                        x >= left && x < left + c.width && y >= c.bottom as f64 && y < c.bottom as f64 + c.height
                    })
                    .count();
                assert_eq!(holding, 1, "level {level} point {x},{y}");
            }
        }
    }
}
