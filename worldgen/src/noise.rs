//! Deterministic hashing, noise and randomness for the world generators:
//! integers wherever they decide something, so a seed gives the same world everywhere and far from the origin.

use std::cell::Cell;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use crate::units::Units;

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

/** A fixed-point number: 16 bits after the point. Untyped: worlds are moving to `Fx`. */
pub type Fixed = i64;
pub const ONE: Fixed = 1 << 16;

/** `x` in fixed point, for constants. */
pub const fn fixed(x: f64) -> Fixed {
    (x * ONE as f64).round() as Fixed
}

/**
 * A fixed-point number, 16 bits after the point: a fraction, a factor, a
 * noise's value, or a length in lattice units with its fraction kept.
 * Unlike `Fixed` it is not an integer: it meets whole `Units` only in
 * `Fx * Units` and `Fx / Units`, and becomes them only by `floor_units`
 * or `round_units`. Products are floored (an arithmetic shift, as the
 * noise eases), quotients truncated towards zero (as `i64`'s `/`).
 */
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Fx(Fixed);

/** `x` as an `Fx`, for constants. */
pub const fn fx(x: f64) -> Fx {
    Fx(fixed(x))
}

impl Fx {
    pub const ZERO: Fx = Fx(0);
    pub const HALF: Fx = Fx(ONE / 2);
    pub const ONE: Fx = Fx(ONE);
    pub const MIN: Fx = Fx(Fixed::MIN);

    /** The `Fx` of an untyped `Fixed`, where typed code meets code not yet typed. */
    pub const fn from_bits(bits: Fixed) -> Fx {
        Fx(bits)
    }

    /** As an untyped `Fixed`, for code not yet typed. */
    pub const fn to_bits(self) -> Fixed {
        self.0
    }

    /** The whole units at or below it. */
    pub const fn floor_units(self) -> Units {
        Units(self.0 >> 16)
    }

    /** The nearest whole units, halves up. */
    pub const fn round_units(self) -> Units {
        Units((self.0 + ONE / 2) >> 16)
    }

    pub fn to_f64(self) -> f64 {
        float(self.0)
    }

    pub const fn abs(self) -> Fx {
        Fx(self.0.abs())
    }

    /** −1, 0 or 1. */
    pub const fn signum(self) -> i64 {
        self.0.signum()
    }

    pub const fn saturating_sub(self, other: Fx) -> Fx {
        Fx(self.0.saturating_sub(other.0))
    }

    /** The mean of `values`, truncated towards zero; zero if there are none. */
    pub fn mean(values: impl IntoIterator<Item = Fx>) -> Fx {
        let (sum, n) = values.into_iter().fold((0, 0), |(sum, n), v| (sum + v.0, n + 1));
        Fx(if n == 0 { 0 } else { sum / n })
    }
}

impl Add for Fx {
    type Output = Fx;
    fn add(self, other: Fx) -> Fx {
        Fx(self.0 + other.0)
    }
}

impl Sub for Fx {
    type Output = Fx;
    fn sub(self, other: Fx) -> Fx {
        Fx(self.0 - other.0)
    }
}

impl Neg for Fx {
    type Output = Fx;
    fn neg(self) -> Fx {
        Fx(-self.0)
    }
}

impl AddAssign for Fx {
    fn add_assign(&mut self, other: Fx) {
        self.0 += other.0;
    }
}

impl SubAssign for Fx {
    fn sub_assign(&mut self, other: Fx) {
        self.0 -= other.0;
    }
}

impl Sum for Fx {
    fn sum<I: Iterator<Item = Fx>>(values: I) -> Fx {
        Fx(values.map(|v| v.0).sum())
    }
}

/** Floored. */
impl Mul for Fx {
    type Output = Fx;
    fn mul(self, other: Fx) -> Fx {
        Fx((self.0 * other.0) >> 16)
    }
}

/** Truncated towards zero. */
impl Div for Fx {
    type Output = Fx;
    fn div(self, other: Fx) -> Fx {
        Fx(self.0 * ONE / other.0)
    }
}

/** Exact: a fraction of whole units is units with a fraction. */
impl Mul<Units> for Fx {
    type Output = Fx;
    fn mul(self, units: Units) -> Fx {
        Fx(self.0 * units.0)
    }
}

impl Mul<Fx> for Units {
    type Output = Fx;
    fn mul(self, x: Fx) -> Fx {
        x * self
    }
}

/** Truncated towards zero: units with a fraction over whole units is a fraction. */
impl Div<Units> for Fx {
    type Output = Fx;
    fn div(self, units: Units) -> Fx {
        Fx(self.0 / units.0)
    }
}

pub(crate) mod sealed {
    /** The raw bits of a fixed-point number, out of reach of the worlds. */
    pub trait Bits: Copy {
        fn bits(self) -> super::Fixed;
        fn of_bits(bits: super::Fixed) -> Self;
    }
}

/** `Fx`, or the untyped `Fixed` it replaces: what the fixed-point helpers take while the worlds move to `Fx`. */
pub trait FixedPoint: sealed::Bits {}

impl sealed::Bits for Fixed {
    fn bits(self) -> Fixed {
        self
    }
    fn of_bits(bits: Fixed) -> Fixed {
        bits
    }
}

impl sealed::Bits for Fx {
    fn bits(self) -> Fixed {
        self.0
    }
    fn of_bits(bits: Fixed) -> Fx {
        Fx(bits)
    }
}

impl FixedPoint for Fixed {}
impl FixedPoint for Fx {}

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

/**
 * Lattice noise in [0, ONE) at (x, y) / PERIOD: random values at multiples
 * of PERIOD, eased between. A constant, so dividing by it is multiplying.
 * The corners of the cell it was last asked in are kept, as neighbouring
 * columns mostly ask in the same.
 */
pub struct ValueNoise<const PERIOD: i64> {
    seed: u64,
    cell: Cell<(i64, i64, [Fixed; 4])>,
}

impl<const PERIOD: i64> ValueNoise<PERIOD> {
    pub fn new(seed: u64) -> Self {
        ValueNoise { seed, cell: Cell::new((i64::MIN, i64::MIN, [0; 4])) }
    }

    #[inline(always)]
    pub fn at(&self, x: i64, y: i64) -> Fixed {
        let (ix, iy) = (x.div_euclid(PERIOD), y.div_euclid(PERIOD));
        let (kx, ky, mut c) = self.cell.get();
        if (kx, ky) != (ix, iy) {
            let at = |i, j| fraction(hash(self.seed, i, j, 0));
            c = [at(ix, iy), at(ix + 1, iy), at(ix, iy + 1), at(ix + 1, iy + 1)];
            self.cell.set((ix, iy, c));
        }
        let u = ease((x.rem_euclid(PERIOD) << 16) / PERIOD);
        let v = ease((y.rem_euclid(PERIOD) << 16) / PERIOD);
        lerp(lerp(c[0], c[1], u), lerp(c[2], c[3], u), v)
    }

    /** `at`, typed. */
    #[inline(always)]
    pub fn fx(&self, x: i64, y: i64) -> Fx {
        Fx(self.at(x, y))
    }
}

/** Fractal value noise in [0, ONE): OCTAVES of it, each at half the period and half the weight of the last. */
pub struct Fbm<const PERIOD: i64, const OCTAVES: usize>([ValueNoise<PERIOD>; OCTAVES]);

impl<const PERIOD: i64, const OCTAVES: usize> Fbm<PERIOD, OCTAVES> {
    pub fn new(seed: u64) -> Self {
        Fbm(std::array::from_fn(|i| ValueNoise::new(stream(seed, i as u64))))
    }

    #[inline(always)]
    pub fn at(&self, x: i64, y: i64) -> Fixed {
        let sum: Fixed = self.0.iter().enumerate().map(|(i, octave)| octave.at(x << i, y << i) << (OCTAVES - 1 - i)).sum();
        sum / ((1 << OCTAVES) - 1)
    }

    /** `at`, typed. */
    #[inline(always)]
    pub fn fx(&self, x: i64, y: i64) -> Fx {
        Fx(self.at(x, y))
    }
}

/** 0 below `edge0`, ONE above `edge1`, eased between. */
pub fn smoothstep<T: FixedPoint>(edge0: T, edge1: T, x: T) -> T {
    let (edge0, edge1, x) = (edge0.bits(), edge1.bits(), x.bits());
    T::of_bits(if x <= edge0 {
        0
    } else if x >= edge1 {
        ONE
    } else {
        ease(((x - edge0) << 16) / (edge1 - edge0))
    })
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
    pub fn chance(&mut self, p: impl FixedPoint) -> bool {
        fraction(self.next_u32()) < p.bits()
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
            let (noise, fbm) = (ValueNoise::<48>::new(3), Fbm::<360, 4>::new(3));
            let mut last = noise.at(x0, 5);
            for x in x0 + 1..x0 + 2000 {
                let v = noise.at(x, 5);
                assert!((0..ONE).contains(&v) && (v - last).abs() < ONE / 16, "{x}: {last} to {v}");
                assert!((0..ONE).contains(&fbm.at(x, 5)));
                last = v;
            }
        }
        // 32-bit coordinates would wrap here.
        assert_ne!(hash(7, 1, 2, 3), hash(7, 1 + (1 << 32), 2, 3));
    }
}
