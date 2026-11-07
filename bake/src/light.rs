//! Light from a voxel's faces, as hosts that light a world hand it to the
//! baker (worldgen's light.rs makes it): per face, the light it sends out,
//! what it reflects and what it glows, linear, in RGB9E5 (three 9-bit
//! mantissas sharing a 5-bit exponent), so a lamp's glare and a moonlit
//! wall both keep their precision. It is the face's colour as seen.

use crate::world::FACE_KINDS;

/** Words per site in a grid's `light`: its index in `cells`, then its faces', by kind (light_words). */
pub const LIGHT_WORDS: usize = 1 + FACE_KINDS;

/** The light each face of a voxel sends out, by kind, linear: 1 is white. */
pub type FaceLight = [[f32; 3]; FACE_KINDS];

/** A site's words in `light`. */
pub fn light_words(index: usize, light: &FaceLight) -> [u32; LIGHT_WORDS] {
    std::array::from_fn(|w| if w == 0 { index as u32 } else { encode(light[w - 1]) })
}

const MANTISSA: i32 = 9;
const BIAS: i32 = 15;

/** The exponent of a positive finite f32: ⌊log₂ v⌋. */
fn exponent(v: f32) -> i32 {
    ((v.to_bits() >> 23) & 255) as i32 - 127
}

/** RGB, at least 0, as RGB9E5; the largest it holds is about 65000. */
pub fn encode(rgb: [f32; 3]) -> u32 {
    let largest = rgb.iter().fold(0f32, |m, c| m.max(*c)).min(65_000.0);
    if largest < f32::MIN_POSITIVE {
        return 0;
    }
    let mut shared = exponent(largest).max(-BIAS - 1) + 1 + BIAS;
    let scale = |e: i32| 2f32.powi(e - BIAS - MANTISSA);
    if (largest / scale(shared) + 0.5).floor() as i32 == 1 << MANTISSA {
        shared += 1;
    }
    let m = rgb.map(|c| ((c.clamp(0.0, 65_000.0) / scale(shared) + 0.5).floor() as u32).min((1 << MANTISSA) - 1));
    m[0] | m[1] << 9 | m[2] << 18 | (shared as u32) << 27
}

pub fn decode(word: u32) -> [f32; 3] {
    let scale = 2f32.powi((word >> 27) as i32 - BIAS - MANTISSA);
    [0, 9, 18].map(|shift| ((word >> shift) & 511) as f32 * scale)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_keeps_its_precision_from_moonlight_to_glare() {
        for v in [0.0, 1e-4, 0.013, 0.5, 1.0, 3.7, 250.0, 9000.0] {
            for rgb in [[v, v, v], [v, v / 3.0, 0.0]] {
                let back = decode(encode(rgb));
                // Nine bits for the largest channel, the others as many below its exponent.
                let largest = rgb.iter().fold(0f32, |m, c| m.max(*c));
                assert!((0..3).all(|c| (back[c] - rgb[c]).abs() <= largest / 256.0), "{rgb:?} came back as {back:?}");
            }
        }
    }
}
