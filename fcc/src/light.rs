//! Light from a voxel's faces, as hosts that light a world hand it to the
//! baker (Baker's `light`; worldgen's light.rs finds it): per face, the
//! light it sends out, what it reflects and what it glows, linear. It is
//! the face's colour as seen.

use crate::FACE_KINDS;

/** The light each face of a voxel sends out, by kind, linear: 1 is white. */
pub type FaceLight = [[f32; 3]; FACE_KINDS];

/** A host's light: that from the faces of the voxel at a site of a grid level, in its units, as tiles have it. */
pub type Lighting = Box<dyn FnMut(usize, [i32; 3]) -> FaceLight>;

/** Light (linear) as a colour: opaque little-endian sRGB RGBA, as palettes are; what is brighter than white, white. */
pub fn colour(light: [f32; 3]) -> u32 {
    let [r, g, b] = light.map(|c| (255.0 * linear_to_srgb(c.clamp(0.0, 1.0))).round() as u32);
    255 << 24 | b << 16 | g << 8 | r
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}
