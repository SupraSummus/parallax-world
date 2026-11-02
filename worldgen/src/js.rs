//! Numbers as the TS generators compute them in V8, so that a world comes
//! out the same, site for site: JS's integer conversions, rounding, and
//! the math functions V8 uses where Rust's differ in the last bit.
//! `pow` is the C library's in both; atan2 and exp are fdlibm's (libm);
//! V8's sin and cos are its own, within an ulp of libm's.

/** ToInt32: `x | 0`. */
pub fn int32(x: f64) -> i32 {
    // Truncated to i64 exactly, then wrapped: the hashes' hot path.
    if x.abs() < 9.0e18 {
        return x as i64 as i32;
    }
    if !x.is_finite() {
        return 0;
    }
    x.trunc().rem_euclid(4294967296.0) as u32 as i32
}

/** ToUint32: `x >>> 0`. */
pub fn uint32(x: f64) -> u32 {
    int32(x) as u32
}

/** Math.round: to the nearest integer, halves up. */
pub fn round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/** `x ** y` */
pub fn pow(x: f64, y: f64) -> f64 {
    x.powf(y)
}

/** Math.hypot as V8 computes it: scaled by the largest, summed with Kahan's compensation. */
pub fn hypot(values: &[f64]) -> f64 {
    let max = values.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if max == 0.0 {
        return 0.0;
    }
    if max.is_infinite() {
        return max;
    }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/** Math.max of a list (−∞ if empty). */
pub fn max(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(f64::NEG_INFINITY, f64::max)
}

/** Math.min of a list (∞ if empty). */
pub fn min(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_wrap_like_js() {
        assert_eq!(int32(4294967296.0 + 5.0), 5);
        assert_eq!(int32(-1.5), -1);
        assert_eq!(int32(2147483648.0), -2147483648);
        assert_eq!(uint32(-1.0), 4294967295);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(hypot(&[3.0, 4.0]), 5.0);
    }
}
