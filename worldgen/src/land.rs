//! Land: space shared out by bids. Each bid is a scalar; each share is
//! one too, a fraction, the shares of a point adding up to one, so at
//! most one is over a half: that one owns the point (`owns`), and the
//! order of the bids is nothing to it. A share's bounds over a box follow
//! from the bids': what is drawn by it is not asked where it is surely
//! none, and where its least is over a half it owns all of a box. No
//! direction is special: a world on a ground keeps its land by column
//! (the ground crate's land.rs). See "Land" in docs/generators.md.

use std::sync::Arc;

use crate::grid::Box3;
use crate::noise::{Fixed, Fx, smoothstep};
use crate::scalar::Scalar;

/** The most bids a land is shared by. */
pub const BIDS: usize = 6;

/** The best of no other bids: far behind any. */
const NONE: Fx = Fx::from_bits(Fixed::MIN / 2);

/**
 * The shares of a point, fractions adding up to one. Small, as a land on
 * a ground keeps one per column: a share is its `Fx`'s bits, which fit in
 * 32 for a fraction.
 */
#[derive(Clone, Copy, Default)]
pub struct Shares([i32; BIDS]);

/** How a land's shares of a point are read: decided there, or kept by what decides them less often (`kept`). */
pub type Read = Arc<dyn Fn([i64; 3]) -> Shares + Send + Sync>;

/** Whether a share owns what it is of, a point or (its least) a box: over a half, which one share at most is. */
pub fn owns(share: Fx) -> bool {
    share > Fx::HALF
}

/**
 * The shares of `bids`, each all of a point `width` ahead of the best of
 * the rest and none as far behind, eased between; decided at each point
 * read, as bids in space are cheap or a point is read once.
 */
pub fn shares<const N: usize>(width: Fx, bids: [Scalar; N]) -> [Scalar; N] {
    kept(width, bids, |decide| decide)
}

/** The shares of `bids` as `shares` says, read as `keep` reads them, given how they are decided: kept by column on a ground. */
pub fn kept<const N: usize>(width: Fx, bids: [Scalar; N], keep: impl FnOnce(Read) -> Read) -> [Scalar; N] {
    const { assert!(N <= BIDS, "a land is shared by BIDS bids at most") };
    let (bids, deciding) = (bids.to_vec(), bids.to_vec());
    let land = Arc::new(Land { width, bids, read: keep(Arc::new(move |p| decide(width, &deciding, p))) });
    std::array::from_fn(|d| {
        let (at, over) = (land.clone(), land.clone());
        Scalar::new(move |p| Fx::from_bits((at.read)(p).0[d].into()), move |b| over.over(d, b))
    })
}

/** The bids, how wide the shares ease, and how they are read. */
struct Land {
    width: Fx,
    bids: Vec<Scalar>,
    read: Read,
}

/** How much a bid `lead` ahead of the best of the rest gets, before the shares are made to add up. */
fn raw(width: Fx, lead: Fx) -> Fx {
    smoothstep(-width, width, lead)
}

/** The shares of point `p` by `bids`: each by how far it leads the best of the rest. */
fn decide(width: Fx, bids: &[Scalar], p: [i64; 3]) -> Shares {
    let n = bids.len();
    let mut bid = [Fx::MIN; BIDS];
    for (d, of) in bids.iter().enumerate() {
        bid[d] = of.at(p);
    }
    let mut raws = [Fx::ZERO; BIDS];
    for d in 0..n {
        let rest = (0..n).filter(|e| *e != d).map(|e| bid[e]).max().unwrap_or(NONE);
        raws[d] = raw(width, bid[d].saturating_sub(rest));
    }
    // The best bid is ahead of the rest or level with them, so the sum is at least a half.
    let sum: Fx = raws.into_iter().sum();
    Shares(raws.map(|r| (r / sum).to_bits() as i32))
}

impl Land {
    /**
     * The least and the most bid `d`'s share is over box `b`: as low as
     * its least bid against the others' most, which would take theirs,
     * and as high the other way round.
     */
    fn over(&self, d: usize, b: &Box3) -> (Fx, Fx) {
        let bids: Vec<(Fx, Fx)> = self.bids.iter().map(|bid| bid.over(b)).collect();
        // Each bid's least and most before the shares add up: against the best of the rest's most, and least.
        let raws: Vec<(Fx, Fx)> = (0..bids.len())
            .map(|e| {
                let rest = bids.iter().enumerate().filter(|(f, _)| *f != e).map(|(_, bid)| *bid);
                let (least, most) = rest.fold((NONE, NONE), |(l, m), (lo, hi)| (l.max(lo), m.max(hi)));
                (raw(self.width, bids[e].0.saturating_sub(most)), raw(self.width, bids[e].1.saturating_sub(least)))
            })
            .collect();
        let (least, most) =
            raws.iter().enumerate().filter(|(e, _)| *e != d).fold((Fx::ZERO, Fx::ZERO), |(l, m), (_, r)| (l + r.0, m + r.1));
        // Its part of all of them; less and more by the bit a quotient's truncation may lose.
        let part = |own: Fx, others: Fx, bit: i64| if own > Fx::ZERO { own / (own + others) + Fx::from_bits(bit) } else { Fx::ZERO };
        (part(raws[d].0, most, -1).max(Fx::ZERO), part(raws[d].1, least, 1).min(Fx::ONE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::fx;

    #[test]
    fn shares_add_up_ease_across_the_edge_and_keep_within_their_bounds() {
        // The first bids x / 100, the second nothing, the third a little less than nothing.
        let bid = |x: i64| Fx::from_bits(x * Fx::ONE.to_bits() / 100);
        let along = Scalar::new(move |p| bid(p[0]), move |b| (bid(b.min[0]), bid(b.max[0] - 1)));
        let shares = shares(fx(0.1), [along, Scalar::constant(Fx::ZERO), Scalar::constant(fx(-0.05))]);
        let line = |x0: i64, x1: i64| Box3 { min: [x0, 0, 0], max: [x1 + 1, 1, 1] };
        for x in -30..30 {
            let s = shares.iter().map(|share| share.at([x, 0, 0])).collect::<Vec<_>>();
            assert!((s.iter().copied().sum::<Fx>() - Fx::ONE).abs() <= Fx::from_bits(2), "{x}: {s:?}");
            assert!(s.iter().filter(|s| owns(**s)).count() <= 1, "{x}: {s:?} owned twice");
            for (d, share) in shares.iter().enumerate() {
                // Within its bounds over any box it is in; owning all of one where its least does.
                for (x0, x1) in [(x, x), (x - 7, x + 3), (x - 40, x + 40)] {
                    let (lo, hi) = share.over(&line(x0, x1));
                    assert!(lo <= s[d] && s[d] <= hi, "{d} at {x}: {:?} not within {lo:?} to {hi:?} over {x0} to {x1}", s[d]);
                    assert!(!owns(lo) || owns(s[d]), "{d} at {x}: surely owns {x0} to {x1}, but not");
                }
            }
        }
        assert_eq!((shares[0].at([-20, 0, 0]), shares[0].at([20, 0, 0])), (Fx::ZERO, Fx::ONE));
        // Surely none only where the bids leave it none, over a box as over its points; surely all where they give it all.
        let may = |d: usize, x0, x1| shares[d].may(&line(x0, x1));
        assert_eq!([may(0, -40, -20), may(0, -40, 0), may(1, 20, 40)], [false, true, false]);
        assert!(owns(shares[0].over(&line(20, 40)).0));
    }
}
