//! Deterministic hashing, noise and randomness for the world generators:
//! integers wherever they decide something, so a seed gives the same world everywhere and far from the origin.

/** splitmix64's finaliser: a bijection of u64 that mixes every bit into every other. */
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/** A hash of a lattice point, 64 bits: the coordinates are mixed in one after another, so the world never repeats. */
pub fn hash64(seed: u64, x: i64, y: i64, z: i64) -> u64 {
    let h = mix(seed ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15));
    let h = mix(h ^ (y as u64).wrapping_mul(0xc2b2ae3d27d4eb4f));
    mix(h ^ (z as u64).wrapping_mul(0x165667b19e3779f9))
}

/** A hash of a lattice point, 32 bits. */
pub fn hash(seed: u64, x: i64, y: i64, z: i64) -> u32 {
    (hash64(seed, x, y, z) >> 32) as u32
}

/** Seed number `n` derived from `seed`, for one of a world's many independent streams. */
pub fn stream(seed: u64, n: u64) -> u64 {
    mix(seed ^ mix(n.wrapping_add(0xd6e8feb86659fd93)))
}

/** A seed from a text and a seed (FNV-1a, then mixed). */
pub fn hash_text(text: &str, seed: u64) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for byte in text.bytes() {
        h = (h ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    mix(h ^ seed)
}

/** A 32-bit hash as a fraction of 1, in [0, 1): exact in an f64. */
pub fn unit(h: u32) -> f64 {
    h as f64 / 4294967296.0
}

/** Smoothly interpolated lattice noise in [0, 1). */
pub fn value_noise(seed: u64, x: f64, y: f64) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = x - ix;
    let fy = y - iy;
    let u = fx * fx * (3.0 - 2.0 * fx);
    let v = fy * fy * (3.0 - 2.0 * fy);
    let (ix, iy) = (ix as i64, iy as i64);
    let a = unit(hash(seed, ix, iy, 0));
    let b = unit(hash(seed, ix + 1, iy, 0));
    let c = unit(hash(seed, ix, iy + 1, 0));
    let d = unit(hash(seed, ix + 1, iy + 1, 0));
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

/** Fractal (multi-octave) value noise in [0, 1). */
pub fn fbm(seed: u64, mut x: f64, mut y: f64, octaves: u32) -> f64 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    for i in 0..octaves {
        sum += value_noise(stream(seed, i as u64), x, y) * amplitude;
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

/** A seeded random sequence (splitmix64). */
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        mix(self.0)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /** A whole number in [0, n), each as likely (to within 2^-32). */
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next_u32() as u64 * n as u64) >> 32) as u32
    }

    /** A fraction of 1 in [0, 1), in steps of 2^-32. */
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        unit(self.next_u32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_do_not_repeat_and_mix_every_bit() {
        let mut seen = std::collections::HashSet::new();
        for x in -16..16 {
            for y in -16..16 {
                for z in -16..16 {
                    assert!(seen.insert(hash64(7, x, y, z)));
                }
            }
        }
        // Far apart, still apart: 2^32 is no period.
        assert_ne!(hash64(7, 1, 2, 3), hash64(7, 1 + (1 << 32), 2, 3));
        // Half the bits flip from one point to the next, on average.
        let flips: u32 = (0..1000).map(|x| (hash64(7, x, 0, 0) ^ hash64(7, x + 1, 0, 0)).count_ones()).sum();
        assert!((31_000..33_000).contains(&flips), "{flips}");
    }

    #[test]
    fn below_is_even() {
        let mut r = Rng::new(1);
        let mut counts = [0; 6];
        for _ in 0..60_000 {
            counts[r.below(6) as usize] += 1;
        }
        assert!(counts.iter().all(|c| (9_500..10_500).contains(c)), "{counts:?}");
    }
}
