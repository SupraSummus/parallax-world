//! The voxels of a chunk cell that can show: its surface.

use crate::cells::Cells;
use crate::slicing::{cell_margin, chunk_cell, site_layers};
use crate::world::{ALPHA, FACE_KINDS, Geometry, Reader};

/** A voxel of a cell's surface. Bit k of `open` and `cut` is face kind k. */
pub struct Voxel {
    /** Lattice coordinates (a, b, c). */
    pub site: [i32; 3],
    /** That of its face towards the camera (Reader). */
    pub material: u8,
    /** Of ALPHA, how much of the rays through its site it stops. */
    pub alpha: u8,
    /** Faces whose neighbour across them lets rays through: none, or a see-through one. */
    pub open: u16,
    /**
     * Faces drawn because the slice is cut there: their neighbour is in
     * another slice. Cells of neighbouring layers interlock, so that may be
     * a farther one, even across a face towards the camera.
     */
    pub cut: u16,
}

/**
 * A slice: slab `layer` of `level`, made of cells of level `atoms` (see
 * SliceId in slice-selection.ts). As its grid sees it (in_grid in lib.rs),
 * of single voxels: the sites in the slab's 2^level layers. If `cutoff` is
 * finite, only those whose cell of 2·FCC has its site before layer `cutoff`.
 */
#[derive(Clone, Copy, PartialEq)]
pub struct Slice {
    pub level: i32,
    pub atoms: i32,
    pub layer: i32,
    pub cutoff: f64,
}

/** The voxels cell_surface looks at for chunk cell (i, j) of a slice, as a box of (a, b, c), inclusive. */
fn cell_box(slice: &Slice, i: i32, j: i32) -> ([i32; 3], [i32; 3]) {
    let cell = chunk_cell(slice.level, i, j);
    let (ma, mb) = cell_margin(slice.level, slice.atoms);
    let start = slice.layer * (1 << slice.level);
    let (first, last) = site_layers(slice.level, slice.atoms);
    ([cell.left - ma, cell.bottom - mb, start + first], [cell.left + cell.columns + ma - 1, cell.bottom + cell.rows + mb - 1, start + last])
}

/**
 * The voxels of chunk cell (i, j) (chunkCell in slice-selection.ts) of
 * a slice, and of the margin around it its chunks may show (cell_margin),
 * that have an open face (one no neighbour covers), or a face whose
 * neighbour is in another slice (the slice is cut there and the cut must
 * show). Buried voxels are dropped.
 * `next` are the cells of 2·FCC (for the cutoff).
 */
pub fn cell_surface(reader: &Reader, geometry: &Geometry, next: &Cells, slice: &Slice, i: i32, j: i32) -> Vec<Voxel> {
    let (from, to) = cell_box(slice, i, j);
    // Its sites are in layers [start, end).
    let start = slice.layer * (1 << slice.level);
    let end = start + (1 << slice.level);
    let holds = |a: i32, b: i32, c: i32| {
        let past_cutoff = slice.cutoff.is_finite() && next.owner_c(a, b, c) as f64 >= slice.cutoff;
        !past_cutoff && (start..end).contains(&c)
    };
    surface(reader, geometry, from, to, holds)
}

/**
 * The voxels of a box of sites (a, b, c), inclusive, that a chunk `holds`
 * and that have an open face (whose neighbour lets rays through), or a face
 * whose neighbour it does not hold (the chunk is cut there and the cut must
 * show).
 */
pub fn surface(reader: &Reader, geometry: &Geometry, from: [i32; 3], to: [i32; 3], holds: impl Fn(i32, i32, i32) -> bool) -> Vec<Voxel> {
    let (min, max) = (reader.min, reader.max);
    let (a0, a1) = (from[0].max(min[0]), (to[0] + 1).min(max[0] + 1));
    let (b0, b1) = (from[1].max(min[1]), (to[1] + 1).min(max[1] + 1));
    let (bottom, top) = (from[2].max(min[2]), to[2].min(max[2]));
    let mut voxels = Vec::new();
    if a0 >= a1 || b0 >= b1 || top < bottom {
        return voxels;
    }

    // Layers as dense planes of the cell and its sites' neighbours (up to 2
    // away in a, 1 in b): site (a, b) at (b - b0 + 1) · stride + a - a0 + 2,
    // its material, its alpha and whether it is in the chunk.
    let stride = a1 - a0 + 4;
    let plane_size = (stride * (b1 - b0 + 2)) as usize;
    let offsets: [isize; FACE_KINDS] = std::array::from_fn(|k| (geometry.offsets[k][1] * stride + geometry.offsets[k][0]) as isize);
    let fill = |plane: &mut Plane, c: i32| {
        plane.0.fill(0);
        plane.1.fill(0);
        // A layer's sites have a and b of the parity of c.
        for b in (b0 - 1 + ((b0 - 1 + c) & 1)..=b1).step_by(2) {
            let row = (b - b0 + 1) * stride - a0 + 2;
            let first = a0 - 2 + ((a0 + c) & 1);
            reader.row(first, b, c, (a1 + 2 - first + 1) / 2, |i, material, alpha| {
                let a = first + 2 * i;
                plane.0[(row + a) as usize] = material;
                plane.1[(row + a) as usize] = alpha;
                if material != 0 {
                    plane.2[(row + a) as usize] = holds(a, b, c);
                }
            });
        }
    };

    // Planes of layers c - 2 to c + 2: a voxel's neighbours.
    type Plane = (Vec<u8>, Vec<u8>, Vec<bool>);
    let mut planes: [Plane; 5] = std::array::from_fn(|_| (vec![0; plane_size], vec![0; plane_size], vec![false; plane_size]));
    for d in -2..=2 {
        fill(&mut planes[(d + 2) as usize], top + d);
    }
    for c in (bottom..=top).rev() {
        if c < top {
            planes.rotate_right(1);
            fill(&mut planes[0], c - 2);
        }
        let current = &planes[2];
        for b in (b0 + ((b0 + c) & 1)..b1).step_by(2) {
            let row = (b - b0 + 1) * stride - a0 + 2;
            for a in (a0 + ((a0 + c) & 1)..a1).step_by(2) {
                let n = (row + a) as isize;
                let (material, alpha) = (current.0[n as usize], current.1[n as usize]);
                if material == 0 || !current.2[n as usize] {
                    continue;
                }
                let mut open = 0;
                let mut cut = 0;
                for k in 0..FACE_KINDS {
                    let (_, alphas, places) = &planes[(geometry.offsets[k][2] + 2) as usize];
                    let m = (n + offsets[k]) as usize;
                    if alphas[m] < ALPHA {
                        open |= 1 << k;
                    } else if !places[m] {
                        cut |= 1 << k;
                    }
                }
                if open == 0 && cut == 0 {
                    continue;
                }
                voxels.push(Voxel { site: [a, b, c], material, alpha, open, cut });
            }
        }
    }
    voxels
}
