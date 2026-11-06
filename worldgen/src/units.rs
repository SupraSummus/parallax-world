//! Lattice units in metres, for the worlds that measure in metres;
//! whole units typed apart from fixed point (`Fx`).

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use crate::noise::{Fx, ONE, fx};

/** Metres per lattice unit. */
pub const UNIT_SIZE: f64 = 0.125;

/** Metres in whole units, rounded. Untyped: see `Units::m`. */
pub const fn m(metres: f64) -> i64 {
    (metres / UNIT_SIZE + if metres < 0.0 { -0.5 } else { 0.5 }) as i64
}

/**
 * Whole lattice units: a height, a distance, a size. Fixed point meets
 * them only as `Fx * Units` (exact, an `Fx`) and `Fx / Units` (a
 * fraction), so a fraction can't be taken for units, nor units for
 * fixed point, as plain `i64`s could:
 *
 * ```compile_fail,E0308
 * use worldgen::noise::{Fbm, Fx};
 * use worldgen::units::Units;
 * let hills = Fbm::<1920, 4>::new(1);
 * // Whole units plus fixed point: as plain `i64`s, one of them 2^16 times off (the flat moor, F11).
 * let heath = Units::m(-2.0) + (hills.at(0, 0) - Fx::HALF) * Units::m(28.0);
 * ```
 *
 * ```compile_fail,E0308
 * use worldgen::noise::{Fbm, Fx};
 * use worldgen::units::Units;
 * let hills = Fbm::<1920, 4>::new(1);
 * // An `Fx`, not whole units.
 * let heath: Units = (hills.at(0, 0) - Fx::HALF) * Units::m(28.0);
 * ```
 *
 * ```
 * use worldgen::noise::{Fbm, Fx};
 * use worldgen::units::Units;
 * let hills = Fbm::<1920, 4>::new(1);
 * let heath: Units = ((hills.at(0, 0) - Fx::HALF) * Units::m(28.0)).round_units();
 * assert!(heath.abs() <= Units::m(14.0));
 * ```
 */
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Units(pub i64);

impl Units {
    /** Metres in whole units, rounded. */
    pub const fn m(metres: f64) -> Units {
        Units(m(metres))
    }

    /** In fixed point, exactly. */
    pub const fn to_fx(self) -> Fx {
        Fx::from_bits(self.0 * ONE)
    }

    pub const fn abs(self) -> Units {
        Units(self.0.abs())
    }

    pub const fn rem_euclid(self, w: Units) -> Units {
        Units(self.0.rem_euclid(w.0))
    }

    /** How many whole `w` fit below it, rounded down. */
    pub const fn div_euclid(self, w: Units) -> i64 {
        self.0.div_euclid(w.0)
    }
}

impl Fx {
    /** Metres in units, their fraction kept. */
    pub const fn m(metres: f64) -> Fx {
        fx(metres / UNIT_SIZE)
    }
}

impl Add for Units {
    type Output = Units;
    fn add(self, other: Units) -> Units {
        Units(self.0 + other.0)
    }
}

impl Sub for Units {
    type Output = Units;
    fn sub(self, other: Units) -> Units {
        Units(self.0 - other.0)
    }
}

impl Neg for Units {
    type Output = Units;
    fn neg(self) -> Units {
        Units(-self.0)
    }
}

impl AddAssign for Units {
    fn add_assign(&mut self, other: Units) {
        self.0 += other.0;
    }
}

impl SubAssign for Units {
    fn sub_assign(&mut self, other: Units) {
        self.0 -= other.0;
    }
}

impl Mul<i64> for Units {
    type Output = Units;
    fn mul(self, n: i64) -> Units {
        Units(self.0 * n)
    }
}

/** Truncated towards zero. */
impl Div<i64> for Units {
    type Output = Units;
    fn div(self, n: i64) -> Units {
        Units(self.0 / n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::fx;

    #[test]
    fn fixed_point_meets_units_exactly_and_rounds_only_when_asked() {
        let (a, u) = (fx(0.3), Units::m(28.0));
        assert_eq!((a * u).to_bits(), a.to_bits() * u.0);
        assert_eq!(a * u, a * u.to_fx());
        assert_eq!((a * u / u, (a * u).floor_units(), (a * u).round_units()), (a, Units(67), Units(67)));
        assert_eq!(((-fx(2.5)).floor_units(), (-fx(2.5)).round_units(), fx(2.5).round_units()), (Units(-3), Units(-2), Units(3)));
        assert_eq!((-fx(2.5) / Units(2), -fx(2.5) * Fx::HALF), (-fx(1.25), -fx(1.25)));
        assert_eq!((Fx::ONE / fx(4.0), Fx::mean([fx(-1.0), Fx::ZERO, Fx::ZERO])), (fx(0.25), Fx::from_bits(-ONE / 3)));
    }
}
