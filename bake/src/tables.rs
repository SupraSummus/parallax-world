//! What the baker makes of the view's faces and the world's colours: the
//! voxel's faces in lattice coordinates, and each face kind's shaded
//! colour per view face.

use crate::slicing::LAYER;
use crate::world::{FACE_KINDS, Geometry, Lattice};

/**
 * The voxel's 12 faces' normals in the world, the offsets of the
 * neighbours across them; a face's index is its kind (VOXEL_FACES in
 * fcc.ts, in the same order).
 */
pub const VOXEL_FACES: [[i32; 3]; FACE_KINDS] = [
    [-1, -1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [1, 1, 0],
    [0, -1, -1],
    [0, -1, 1],
    [0, 1, -1],
    [0, 1, 1],
    [-1, 0, -1],
    [-1, 0, 1],
    [1, 0, -1],
    [1, 0, 1],
];

/** A world vector in a face's lattice coordinates (a, b, c) = (v · R, v · U, v · F). */
fn lattice_of(Lattice { r, u, f }: &Lattice, v: [i32; 3]) -> [i32; 3] {
    [r, u, f].map(|axis| axis[0] * v[0] + axis[1] * v[1] + axis[2] * v[2])
}

/**
 * Per face kind, the neighbour's offset (a, b, c) in a face's lattice
 * coordinates, and the normal in face coordinates (a · LAYER, b, c · LAYER):
 * the same for every face, as they see the same lattice.
 */
pub fn geometry(lattice: &Lattice) -> Geometry {
    let offsets = VOXEL_FACES.map(|n| lattice_of(lattice, n));
    Geometry { offsets, normals: offsets.map(|[a, b, c]| [a as f64 * LAYER, b as f64, c as f64 * LAYER]) }
}

/**
 * Brightness of a face by its outward normal in the world: the axes' shades
 * weighted by the squared components, so a face between the top and the
 * front gets the mean of both.
 */
fn face_shade([x, y, z]: [f64; 3]) -> f64 {
    let (front, back, left, right, bottom, top) = (0.9, 0.65, 0.7, 0.8, 0.55, 1.0);
    let side = |v: f64, negative: f64, positive: f64| v * v * if v < 0.0 { negative } else { positive };
    (side(x, left, right) + side(y, bottom, top) + side(z, front, back)) / (x * x + y * y + z * z)
}

/**
 * Per view face, the colour of each (material, face kind) at index
 * material · FACE_KINDS + kind: the material's colour (0xrrggbb) times the
 * kind's shade the way it faces in the world, as opaque little-endian RGBA.
 */
pub fn palettes(lattices: &[Lattice], colours: &[u32]) -> Vec<u32> {
    let mut all = Vec::with_capacity(lattices.len() * colours.len() * FACE_KINDS);
    for Lattice { r, u, f } in lattices {
        let shades = VOXEL_FACES.map(|n| {
            let [a, b, c] = lattice_of(&lattices[0], n).map(f64::from);
            face_shade(std::array::from_fn(|d| (a * r[d] as f64 + c * f[d] as f64) / 2.0 + b * u[d] as f64))
        });
        for rgb in colours {
            for shade in shades {
                // Halves rounded up.
                let channel = |shift: u32| (((rgb >> shift) & 255) as f64 * shade + 0.5).floor() as u32;
                all.push(255 << 24 | channel(0) << 16 | channel(8) << 8 | channel(16));
            }
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shades_a_face_by_the_axes_it_faces_along_in_the_world() {
        let [front, left, top] = [[0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].map(face_shade);
        // Seen along +x+z (up is +y, right is (1, 0, −1) / √2): the front
        // kind faces −x and −z; the kind a step up, right and towards the
        // camera faces up and −z.
        let lattice = Lattice { r: [1, 0, -1], u: [0, 1, 0], f: [1, 0, 1] };
        let palette = palettes(&[lattice], &[0, 0xff0000]);
        let kind = |offset: [i32; 3]| VOXEL_FACES.iter().position(|&n| lattice_of(&lattice, n) == offset).expect("a kind");
        let red = |k: usize| palette[FACE_KINDS + k] & 255;
        assert_eq!(red(kind([0, 0, -2])), (255.0 * (left + front) / 2.0 + 0.5).floor() as u32);
        assert_eq!(red(kind([1, 1, -1])), (255.0 * (top + front) / 2.0 + 0.5).floor() as u32);
    }
}
