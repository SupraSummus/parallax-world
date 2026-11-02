//! Deterministic noise and randomness shared by the world generators (noise.ts).

use crate::js::int32;

/** Integer hash of a lattice point to [0, 1). */
pub fn hash(seed: f64, x: f64, y: f64, z: f64) -> f64 {
    let h = int32(seed)
        .wrapping_add(int32(x).wrapping_mul(374761393))
        .wrapping_add(int32(y).wrapping_mul(668265263))
        .wrapping_add(int32(z).wrapping_mul(1440662683));
    let h = (h ^ ((h as u32) >> 13) as i32).wrapping_mul(1274126177);
    let h = h ^ ((h as u32) >> 16) as i32;
    h as u32 as f64 / 4294967296.0
}

/** Smoothly interpolated lattice noise in [0, 1). */
pub fn value_noise(seed: f64, x: f64, y: f64) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = x - ix;
    let fy = y - iy;
    let u = fx * fx * (3.0 - 2.0 * fx);
    let v = fy * fy * (3.0 - 2.0 * fy);
    let a = hash(seed, ix, iy, 0.0);
    let b = hash(seed, ix + 1.0, iy, 0.0);
    let c = hash(seed, ix, iy + 1.0, 0.0);
    let d = hash(seed, ix + 1.0, iy + 1.0, 0.0);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

/** Fractal (multi-octave) value noise in [0, 1). */
pub fn fbm(seed: f64, mut x: f64, mut y: f64, octaves: u32) -> f64 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    for i in 0..octaves {
        sum += value_noise(seed + i as f64 * 7919.0, x, y) * amplitude;
        total += amplitude;
        amplitude /= 2.0;
        x *= 2.0;
        y *= 2.0;
    }
    sum / total
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = 1f64.min(0f64.max((x - edge0) / (edge1 - edge0)));
    t * t * (3.0 - 2.0 * t)
}

/** Seeded PRNG (mulberry32) returning [0, 1). */
#[derive(Clone)]
pub struct Rng(i32);

impl Rng {
    pub fn new(seed: f64) -> Rng {
        Rng(int32(seed))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let a = self.0;
        let mut t = (a ^ ((a as u32) >> 15) as i32).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ ((t as u32) >> 7) as i32).wrapping_mul(61 | t)) ^ t;
        (t ^ ((t as u32) >> 14) as i32) as u32 as f64 / 4294967296.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values from noise.ts in Node.
    #[test]
    fn matches_noise_ts() {
        assert_eq!(hash(12345.0, 3.0, -7.0, 11.0), 0.3755833769682795);
        let mut r = Rng::new(42.0);
        assert_eq!(r.next(), 0.6011037519201636);
        assert_eq!(r.next(), 0.44829055899754167);
        assert_eq!(fbm(99.0, 1.25, -3.5, 4), 0.55716453428419);
    }
}
