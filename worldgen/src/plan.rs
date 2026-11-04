//! Plans: what is where, decided column by column (x, z), each at a
//! scale and from coarser ones only; and land that domains bid for.
//! See "Composing" in docs/generators.md.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::noise::{Fixed, Fx, smoothstep};

const DOCS: &str = "docs/generators.md";

thread_local! {
    /** The plan being made on this thread and its scale (none: a layer, which may read any). */
    static MAKING: Cell<(&'static str, i64)> = const { Cell::new(("", 0)) };
}

/** Slots a plan keeps values in: as many as two tiles have columns, so a layer finds what the one before made in its tile. */
const SLOTS: usize = 1 << 15;

/**
 * A decision for any column, made on demand and kept (the last made in
 * each slot). Its `scale` is the size of the smallest features it decides,
 * in lattice units; it reads only plans at least as coarse, and panics
 * otherwise, naming both.
 */
pub struct Plan<T> {
    name: &'static str,
    scale: i64,
    /** Made from finer plans' samples: a filter, which may read any. */
    filter: bool,
    slots: RefCell<Vec<(i64, i64, T)>>,
    make: Box<dyn Fn(i64, i64) -> T>,
}

impl<T: Copy + Default> Plan<T> {
    pub fn new(name: &'static str, scale: i64, make: impl Fn(i64, i64) -> T + 'static) -> Plan<T> {
        Plan { name, scale, filter: false, slots: RefCell::new(vec![(i64::MIN, 0, T::default()); SLOTS]), make: Box::new(make) }
    }

    pub fn at(&self, x: i64, z: i64) -> T {
        let (reader, finest) = MAKING.get();
        assert!(
            self.scale >= finest,
            "plan {reader} (scale {finest}) reads {} of the finer scale {}; see \"Bands\" in {DOCS}",
            self.name,
            self.scale
        );
        let slot = (((x as u64).wrapping_mul(0x9e3779b97f4a7c15) ^ (z as u64).wrapping_mul(0xc2b2ae3d27d4eb4f)) >> 49) as usize;
        let (kx, kz, value) = self.slots.borrow()[slot];
        if (kx, kz) == (x, z) {
            return value;
        }
        let outer = MAKING.replace(if self.filter { ("", 0) } else { (self.name, self.scale) });
        let value = (self.make)(x, z);
        MAKING.set(outer);
        self.slots.borrow_mut()[slot] = (x, z, value);
        value
    }
}

impl Plan<Fx> {
    /**
     * A smooth plan made only every `step` columns and interpolated
     * between: a band `step` or more wide costs a column a sample's share.
     * Sampling is what makes a coarse band of finer ones, so `make` may
     * read finer plans; what it reads finer than `step` aliases.
     */
    pub fn sampled(name: &'static str, scale: i64, step: i64, make: impl Fn(i64, i64) -> Fx + 'static) -> Plan<Fx> {
        let mut samples = Plan::new(name, scale, move |i, k| make(i * step, k * step));
        samples.filter = true;
        Plan::new(name, scale, move |x, z| {
            let (i, k) = (x.div_euclid(step), z.div_euclid(step));
            let (u, v) = ((x.rem_euclid(step) << 16) / step, (z.rem_euclid(step) << 16) / step);
            let lerp = |a: Fixed, b: Fixed, t: Fixed| a + (((b - a) * t) >> 16);
            let corner = |i, k| samples.at(i, k).to_bits();
            Fx::from_bits(lerp(lerp(corner(i, k), corner(i + 1, k), u), lerp(corner(i, k + 1), corner(i + 1, k + 1), u), v))
        })
    }
}

/** The most domains a land is shared by. */
pub const DOMAINS: usize = 4;

/** Domains' shares of a column, fractions adding up to one, and the one with the most. */
#[derive(Clone, Copy, Default)]
struct Shares {
    share: [Fx; DOMAINS],
    winner: u8,
}

/**
 * Land shared by domains that never see each other: each bids for every
 * column (a plan of fractions), and gets a share by how far it
 * outbids the best of the rest: all of it `width` ahead, none as far
 * behind, eased between. Whatever the domains propose for a column
 * (heights, densities) blends by these shares.
 */
pub struct Land {
    shares: Plan<Shares>,
}

impl Land {
    /** Land of the domains bidding `bids`, in that order (their indices); `scale` at most the bids'. */
    pub fn new(scale: i64, width: Fx, bids: Vec<Rc<Plan<Fx>>>) -> Land {
        assert!(bids.len() <= DOMAINS, "a land holds at most {DOMAINS} domains; see \"Composing\" in {DOCS}");
        Land {
            shares: Plan::new("land", scale, move |x, z| {
                let mut bid = [Fx::MIN; DOMAINS];
                for (d, plan) in bids.iter().enumerate() {
                    bid[d] = plan.at(x, z);
                }
                let mut raw = [Fx::ZERO; DOMAINS];
                for d in 0..bids.len() {
                    let rest = (0..bids.len()).filter(|e| *e != d).map(|e| bid[e]).max().unwrap_or(Fx::from_bits(Fixed::MIN / 2));
                    raw[d] = smoothstep(-width, width, bid[d].saturating_sub(rest));
                }
                // The best bid is ahead of the rest or level with them, so the sum is at least a half.
                let sum: Fx = raw.into_iter().sum();
                let share = raw.map(|r| r / sum);
                let winner = (0..bids.len()).max_by_key(|d| (share[*d], -(*d as i64))).unwrap_or(0) as u8;
                Shares { share, winner }
            }),
        }
    }

    /** Domain `d`'s share of column (x, z), a fraction. */
    pub fn share(&self, d: usize, x: i64, z: i64) -> Fx {
        self.shares.at(x, z).share[d]
    }

    /** Whether domain `d` has the most of column (x, z): the first of those with the most. */
    pub fn owns(&self, d: usize, x: i64, z: i64) -> bool {
        self.shares.at(x, z).winner as usize == d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{ONE, fx};
    use crate::units::Units;

    #[test]
    #[should_panic(expected = "plan coarse (scale 64) reads fine of the finer scale 1")]
    fn refuses_a_plan_reading_a_finer_one() {
        let fine = Rc::new(Plan::new("fine", 1, |x, _| x));
        let coarse = Plan::new("coarse", 64, move |x, z| fine.at(x, z));
        coarse.at(3, 4);
    }

    #[test]
    fn a_sampled_plan_may_read_finer_ones_and_interpolates() {
        let fine = Rc::new(Plan::new("fine", 1, |x, z| Units(x + 2 * z).to_fx()));
        let coarse = Plan::sampled("coarse", 64, 16, move |x, z| fine.at(x, z));
        assert_eq!((coarse.at(0, 0), coarse.at(8, 0), coarse.at(5, 3)), (Fx::ZERO, fx(8.0), fx(11.0)));
    }

    #[test]
    fn shares_add_up_and_ease_across_the_edge() {
        let a = Rc::new(Plan::new("a", 8, |x, _| Fx::from_bits(x * ONE / 100)));
        let b = Rc::new(Plan::new("b", 8, |_, _| Fx::ZERO));
        let land = Land::new(8, fx(0.1), vec![a, b]);
        for x in -30..30 {
            let (sa, sb) = (land.share(0, x, 0), land.share(1, x, 0));
            assert!((sa + sb - Fx::ONE).abs() <= Fx::from_bits(1), "{x}: {sa:?} + {sb:?}");
            assert_eq!(land.owns(0, x, 0), sa >= sb, "{x}");
        }
        assert_eq!((land.share(0, -20, 0), land.share(0, 20, 0)), (Fx::ZERO, Fx::ONE));
    }
}
