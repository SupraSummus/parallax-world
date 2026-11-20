//! The voxels of a chunk cell that can show: its surface.

use fcc::{ALPHA, FACE_KINDS, Look, Reader, alpha_of};

use crate::tables::Geometry;

/** A voxel of a cell's surface. Bit k of `open` and `cut` is face kind k. */
pub struct Voxel {
    /** Lattice coordinates (a, b, c). */
    pub site: [i32; 3],
    /** Its look through its face towards the camera (Reader). */
    pub look: Look,
    /** Faces whose neighbour across them lets rays through: none, or a see-through one. */
    pub open: u16,
    /**
     * Faces drawn because the chunk is cut there: their neighbour is in
     * another chunk. Cells of neighbouring layers interlock, so that may be
     * a farther one, even across a face towards the camera.
     */
    pub cut: u16,
}

impl Voxel {
    /** Of ALPHA, how much of the rays through its site it stops. */
    pub fn alpha(&self) -> u8 {
        alpha_of(self.look)
    }
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
    // its look and whether it is in the chunk.
    let stride = a1 - a0 + 4;
    let plane_size = (stride * (b1 - b0 + 2)) as usize;
    let offsets: [isize; FACE_KINDS] = std::array::from_fn(|k| (geometry.offsets[k][1] * stride + geometry.offsets[k][0]) as isize);
    let fill = |plane: &mut Plane, c: i32| {
        plane.0.fill(0);
        // A layer's sites have a and b of the parity of c.
        for b in (b0 - 1 + ((b0 - 1 + c) & 1)..=b1).step_by(2) {
            let row = (b - b0 + 1) * stride - a0 + 2;
            let first = a0 - 2 + ((a0 + c) & 1);
            reader.row(first, b, c, (a1 + 2 - first + 1) / 2, |i, look| {
                let a = first + 2 * i;
                plane.0[(row + a) as usize] = look;
                if look != 0 {
                    plane.1[(row + a) as usize] = holds(a, b, c);
                }
            });
        }
    };

    // Planes of layers c - 2 to c + 2: a voxel's neighbours.
    type Plane = (Vec<Look>, Vec<bool>);
    let mut planes: [Plane; 5] = std::array::from_fn(|_| (vec![0; plane_size], vec![false; plane_size]));
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
                let look = current.0[n as usize];
                if look == 0 || !current.1[n as usize] {
                    continue;
                }
                let mut open = 0;
                let mut cut = 0;
                for k in 0..FACE_KINDS {
                    let (looks, places) = &planes[(geometry.offsets[k][2] + 2) as usize];
                    let m = (n + offsets[k]) as usize;
                    if alpha_of(looks[m]) < ALPHA {
                        open |= 1 << k;
                    } else if !places[m] {
                        cut |= 1 << k;
                    }
                }
                if open == 0 && cut == 0 {
                    continue;
                }
                voxels.push(Voxel { site: [a, b, c], look, open, cut });
            }
        }
    }
    voxels
}
