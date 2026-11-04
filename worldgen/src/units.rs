//! Lattice units in metres, for the worlds that measure in metres.

/** Metres per lattice unit. */
pub const UNIT_SIZE: f64 = 0.125;

/** Metres in whole units, rounded. */
pub const fn m(metres: f64) -> i64 {
    (metres / UNIT_SIZE + if metres < 0.0 { -0.5 } else { 0.5 }) as i64
}
