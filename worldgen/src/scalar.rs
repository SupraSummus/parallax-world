//! Scalars: a number at every point, and the least and the most it is
//! over any box, a rule. What says where something may be is one: a
//! bid for land, a share of it, how much a forest thins. A field or a
//! population is drawn within one: not asked where it is surely none.
//! See "Land" in docs/generators.md.

use std::sync::Arc;

use crate::generator::{Field, Meets, Population};
use crate::grid::Box3;
use crate::noise::Fx;

/** A scalar's least and most over a box. */
type Over = dyn Fn(&Box3) -> (Fx, Fx) + Send + Sync;

/** A number at each point (lattice units), and its least and most over each box. */
#[derive(Clone)]
pub struct Scalar {
    at: Arc<dyn Fn([i64; 3]) -> Fx + Send + Sync>,
    over: Arc<Over>,
}

impl Scalar {
    /**
     * The scalar `at` each point, no less than `over(b).0` nor more than
     * `over(b).1` anywhere in box `b`: a rule.
     */
    pub fn new(at: impl Fn([i64; 3]) -> Fx + Send + Sync + 'static, over: impl Fn(&Box3) -> (Fx, Fx) + Send + Sync + 'static) -> Scalar {
        Scalar { at: Arc::new(at), over: Arc::new(over) }
    }

    /** `c` everywhere. */
    pub fn constant(c: Fx) -> Scalar {
        Scalar::new(move |_| c, move |_| (c, c))
    }

    /** It at point `p`. */
    pub fn at(&self, p: [i64; 3]) -> Fx {
        (self.at)(p)
    }

    /** Its least and its most over box `b`. */
    pub fn over(&self, b: &Box3) -> (Fx, Fx) {
        (self.over)(b)
    }

    /**
     * It times `other`: a share thinned. Over a box the least and the most
     * of the products of their bounds; none where it is surely none, the
     * other not asked.
     */
    pub fn times(&self, other: &Scalar) -> Scalar {
        let (a, b) = (self.clone(), other.clone());
        let (c, d) = (self.clone(), other.clone());
        Scalar::new(
            move |p| product(a.at(p), b.at(p)),
            move |x| match c.over(x) {
                (Fx::ZERO, Fx::ZERO) => (Fx::ZERO, Fx::ZERO),
                (lo, hi) => {
                    let (low, high) = d.over(x);
                    let corners = [product(lo, low), product(lo, high), product(hi, low), product(hi, high)];
                    (corners.into_iter().min().unwrap_or(Fx::ZERO), corners.into_iter().max().unwrap_or(Fx::ZERO))
                }
            },
        )
    }

    /** Whether it may be over none somewhere in box `b`. */
    pub fn may(&self, b: &Box3) -> bool {
        self.over(b).1 > Fx::ZERO
    }

    /** Whether it may be over none within `reach` of box `b`: asked of the box itself first, as the smaller is the cheaper to ask. */
    pub fn may_reach(&self, b: &Box3, reach: i64) -> bool {
        self.may(b) || (reach > 0 && self.may(&b.widen(reach)))
    }
}

/** `a` times `b`, floored as `Fx`'s product is, as near as it fits: bounds may be anything. */
fn product(a: Fx, b: Fx) -> Fx {
    let bits = (i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 16;
    Fx::from_bits(bits.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

impl Field {
    /**
     * It drawing only within `reach` of where `scalar` may be over none
     * (Field::meets): what is drawn by what lies as far around it.
     */
    pub fn within(self, scalar: Scalar, reach: i64) -> Field {
        self.meets(move |b, _| if scalar.may_reach(b, reach) { Meets::Any } else { Meets::Nothing })
    }
}

impl Population {
    /** It anchoring things only within `reach` of where `scalar` may be over none (Population::stands). */
    pub fn within(self, scalar: Scalar, reach: i64) -> Population {
        self.stands(move |b| scalar.may_reach(b, reach))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::fx;

    #[test]
    fn a_product_keeps_within_its_bounds_of_any_sign() {
        // x / 8 and 3 − z / 8, over boxes on both sides of none.
        let along = |a: usize, k: f64, c: f64| {
            let f = move |v: i64| fx(c + k * v as f64 / 8.0);
            Scalar::new(move |p| f(p[a]), move |b| (f(b.min[a]).min(f(b.max[a] - 1)), f(b.min[a]).max(f(b.max[a] - 1))))
        };
        let product = along(0, 1.0, 0.0).times(&along(2, -1.0, 3.0));
        for (min, max) in [([-40, 0, -40], [40, 1, 40]), ([3, 0, 30], [9, 1, 50]), ([-9, 0, -9], [-2, 1, 60])] {
            let b = Box3 { min, max };
            let (lo, hi) = product.over(&b);
            for x in min[0]..max[0] {
                for z in min[2]..max[2] {
                    let v = product.at([x, 0, z]);
                    assert!(lo <= v && v <= hi, "({x}, {z}): {v:?} not within {lo:?} to {hi:?}");
                }
            }
        }
        // None times anything, unbounded, is none.
        let anything = Scalar::new(|_| Fx::MAX, |_| (Fx::MIN, Fx::MAX));
        assert_eq!(Scalar::constant(Fx::ZERO).times(&anything).over(&Box3 { min: [0; 3], max: [1; 3] }), (Fx::ZERO, Fx::ZERO));
    }
}
