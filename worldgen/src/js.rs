//! Float math that gives the same bits on every target (native and
//! WebAssembly): libm's functions, not the platform's. Named after JS's
//! Math, as the generators were ported from TS; going as they move to
//! integers.

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
    libm::pow(x, y)
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
    fn rounds_like_js() {
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(hypot(&[3.0, 4.0]), 5.0);
    }
}
