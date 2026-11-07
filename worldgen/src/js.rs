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

/** Math.log: the natural logarithm. */
pub fn log(x: f64) -> f64 {
    libm::log(x)
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

/**
 * `((x % m) + m) % m` for m > 0, exactly, without fmod's slow loop: x % m
 * is x − trunc(x / m)·m, which a fused multiply-add gives exactly unless
 * the quotient rounded off by one, and then it falls outside the range.
 */
pub fn wrap(x: f64, m: f64) -> f64 {
    let r = (-(x / m).trunc()).mul_add(m, x);
    let mut r = if r.abs() < m && (r == 0.0 || (r < 0.0) == (x < 0.0)) { r } else { x % m } + m;
    while r >= m {
        r -= m;
    }
    r
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

    #[test]
    fn wraps_like_js() {
        let mut random = crate::noise::Rng::new(5);
        for _ in 0..1_000_000 {
            let m = 0.1 + random.next() * 3.0;
            let x = (random.next() - 0.5) * [2.0, 100.0, 1e6, 1e18][random.below(4) as usize];
            for x in [x, (x / m).round() * m, (x / m).round() * m + 1e-12] {
                assert_eq!(wrap(x, m).to_bits(), (((x % m) + m) % m).to_bits(), "{x} {m}");
            }
        }
    }
}
