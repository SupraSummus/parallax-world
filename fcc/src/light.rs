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

/**
 * Light (linear) in a word, as GPUs' RGB9E5 takes it: 9 bits a channel
 * from bit 0, times 2 to the power of the top 5 bits less 24. 0 is none.
 */
pub fn rgb9e5(light: [f32; 3]) -> u32 {
    let light = light.map(|c| c.clamp(0.0, 65408.0));
    let max = light.iter().fold(0.0f32, |m, &c| m.max(c));
    // The exponent of the largest, 2^(e − 24) a step of its 9 bits, rounded up a step if it would round to 512 of them.
    let mut e = (max.log2().floor() as i32).max(-16) + 16;
    if (max / (e as f32 - 24.0).exp2()).round() >= 512.0 {
        e += 1;
    }
    let [r, g, b] = light.map(|c| (c / (e as f32 - 24.0).exp2()).round() as u32);
    r | g << 9 | b << 18 | (e as u32) << 27
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb9e5_keeps_light_to_a_step_of_its_brightest_channel() {
        // As voxels.wgsl reads it.
        let read = |w: u32| [0, 9, 18].map(|shift| ((w >> shift) & 511) as f32 * ((w >> 27) as f32 - 24.0).exp2());
        assert_eq!(rgb9e5([0.0; 3]), 0);
        for light in [[1.0, 0.5, 0.25], [0.004, 0.003, 0.0], [2.6, 2.4, 2.1], [250.0, 1.0, 0.0], [0.511, 0.9999, 0.001]] {
            let max = light.iter().fold(0.0f32, |m, &c| m.max(c));
            let back = read(rgb9e5(light));
            assert!((0..3).all(|c| (back[c] - light[c]).abs() <= max / 512.0), "{light:?} read back as {back:?}");
        }
    }
}
