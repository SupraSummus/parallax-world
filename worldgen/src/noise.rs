//! Deterministic hashing, noise and randomness for the world generators:
//! integers wherever they decide something, so a seed gives the same world everywhere and far from the origin.

/** splitmix64's finaliser: a bijection of u64 that mixes every bit into every other. */
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/** A hash of a lattice point, 64 bits: each coordinate mixed in whole, so nothing repeats every 2^32 units. */
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

/** A fixed-point number: 16 bits after the point. */
pub type Fixed = i64;
pub const ONE: Fixed = 1 << 16;

/** `x` in fixed point, for constants. */
pub const fn fixed(x: f64) -> Fixed {
    (x * ONE as f64).round() as Fixed
}

/** `x` as a float, for what is still computed in floats. */
pub fn float(x: Fixed) -> f64 {
    x as f64 / ONE as f64
}

/** A 32-bit hash as a fraction of 1, in [0, ONE). */
pub fn fraction(h: u32) -> Fixed {
    (h >> 16) as Fixed
}

/** 3t² − 2t³: from 0 to ONE as t goes from 0 to ONE, flat at both ends. */
fn ease(t: Fixed) -> Fixed {
    (((t * t) >> 16) * (3 * ONE - 2 * t)) >> 16
}

fn lerp(a: Fixed, b: Fixed, t: Fixed) -> Fixed {
    a + (((b - a) * t) >> 16)
}

/** Lattice noise in [0, ONE) at (x, y) / period: random values at multiples of `period`, eased between. */
pub fn value_noise(seed: u64, x: i64, y: i64, period: i64) -> Fixed {
    let (ix, iy) = (x.div_euclid(period), y.div_euclid(period));
    let u = ease((x.rem_euclid(period) << 16) / period);
    let v = ease((y.rem_euclid(period) << 16) / period);
    let at = |i, j| fraction(hash(seed, i, j, 0));
    lerp(lerp(at(ix, iy), at(ix + 1, iy), u), lerp(at(ix, iy + 1), at(ix + 1, iy + 1), u), v)
}

/** Fractal value noise in [0, ONE): `octaves` of it, each at half the period and half the weight of the last. */
pub fn fbm(seed: u64, x: i64, y: i64, period: i64, octaves: u32) -> Fixed {
    let sum: Fixed = (0..octaves).map(|i| value_noise(stream(seed, i as u64), x << i, y << i, period) << (octaves - 1 - i)).sum();
    sum / ((1 << octaves) - 1)
}

/** 0 below `edge0`, ONE above `edge1`, eased between. */
pub fn smoothstep(edge0: Fixed, edge1: Fixed, x: Fixed) -> Fixed {
    if x <= edge0 {
        0
    } else if x >= edge1 {
        ONE
    } else {
        ease(((x - edge0) << 16) / (edge1 - edge0))
    }
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

    /** True with chance `p` (a fraction of ONE). */
    pub fn chance(&mut self, p: Fixed) -> bool {
        fraction(self.next_u32()) < p
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
    fn noise_holds_up_far_from_the_origin() {
        for x0 in [-1000, 1 << 40] {
            let mut last = value_noise(3, x0, 5, 48);
            for x in x0 + 1..x0 + 2000 {
                let v = value_noise(3, x, 5, 48);
                assert!((0..ONE).contains(&v) && (v - last).abs() < ONE / 16, "{x}: {last} to {v}");
                assert!((0..ONE).contains(&fbm(3, x, 5, 360, 4)));
                last = v;
            }
        }
        // 32-bit coordinates would wrap here.
        assert_ne!(hash(7, 1, 2, 3), hash(7, 1 + (1 << 32), 2, 3));
    }
}
