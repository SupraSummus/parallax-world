//! What the baker is given besides the grids (bake.ts): the voxel's faces in
//! lattice coordinates, and each face kind's shaded colours per view face.

use worldgen::generator::Material;

use crate::slicing::LAYER;
use crate::view::Face;

/** The voxel's 12 faces' normals, the offsets of the neighbours across them, in the order of VOXEL_FACES in fcc.ts. */
fn normals() -> Vec<[i32; 3]> {
    let mut normals = Vec::new();
    for [i, j] in [[0, 1], [1, 2], [0, 2]] {
        for a in [-1, 1] {
            for b in [-1, 1] {
                let mut n = [0; 3];
                n[i] = a;
                n[j] = b;
                normals.push(n);
            }
        }
    }
    normals
}

fn dot(a: [i32; 3], b: [i32; 3]) -> i32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/** A world vector in a face's lattice coordinates (a, b, c) = (v · R, v · U, v · F). */
fn lattice_of(face: &Face, v: [i32; 3]) -> [i32; 3] {
    face.lattice.map(|axis| dot(axis, v))
}

/** VOXEL_GEOMETRY: per face kind, the neighbour's offset (a, b, c) in any face's lattice coordinates, and the normal in face coordinates. */
pub fn geometry(faces: &[Face]) -> Vec<f64> {
    normals()
        .into_iter()
        .flat_map(|n| {
            let [a, b, c] = lattice_of(&faces[0], n);
            [a as f64, b as f64, c as f64, a as f64 * LAYER, b as f64, c as f64 * LAYER]
        })
        .collect()
}

/** Brightness of a face by its outward normal in the world: the axes' shades weighted by the squared components (faceShade in bake.ts). */
fn face_shade([x, y, z]: [f64; 3]) -> f64 {
    let (front, back, left, right, bottom, top) = (0.9, 0.65, 0.7, 0.8, 0.55, 1.0);
    let side = |v: f64, negative: f64, positive: f64| v * v * if v < 0.0 { negative } else { positive };
    (side(x, left, right) + side(y, bottom, top) + side(z, front, back)) / (x * x + y * y + z * z)
}

/**
 * Per view face, the colour of each (material, face kind) at index
 * material · 12 + kind: the material's colour times the kind's shade the way
 * it faces in the world, as opaque little-endian RGBA (shadedPalette in bake.ts).
 */
pub fn palettes(faces: &[Face], materials: &[Material]) -> Vec<u32> {
    let mut all = Vec::new();
    for face in faces {
        let [r, u, f] = face.lattice;
        let shades: Vec<f64> = normals()
            .into_iter()
            .map(|n| {
                let [a, b, c] = lattice_of(&faces[0], n).map(f64::from);
                face_shade(std::array::from_fn(|d| (a * r[d] as f64 + c * f[d] as f64) / 2.0 + b * u[d] as f64))
            })
            .collect();
        for m in materials {
            let rgb = u32::from_str_radix(m.color.get(1..7).unwrap_or("000000"), 16).expect("a #rrggbb colour");
            for shade in &shades {
                // Math.round: halves up.
                let channel = |shift: u32| (((rgb >> shift) & 255) as f64 * shade + 0.5).floor() as u32;
                all.push(255 << 24 | channel(0) << 16 | channel(8) << 8 | channel(16));
            }
        }
    }
    all
}
