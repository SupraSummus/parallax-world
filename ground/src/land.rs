//! Land on a ground: the lie of the land, its height made band by band,
//! each the one before pulled by what is on it; and land shared by bids
//! for columns (x, z), as worldgen's land.rs shares out space, kept by
//! column, its shares read by column. See "Land" and "Ground" in
//! docs/generators.md.

use std::sync::Arc;

use worldgen::grid::Box3;
use worldgen::land;
use worldgen::noise::{Fx, smoothstep};
use worldgen::scalar::Scalar;
use worldgen::units::Units;

use crate::DOCS;
use crate::plan::{Kept, Plan, Sloped, slot};

/**
 * What pulls the ground of a band towards heights of its own (Lie::shaped):
 * a town levelling its streets, the railway its formation.
 */
pub trait Pull: Send + Sync {
    /** How it pulls ground `ground` high at column (x, z): towards a height, and how hard, a fraction; or not at all. */
    fn pull(&self, x: i64, z: i64, ground: Fx) -> Option<(Fx, Fx)>;
}

/** A scalar of columns: `at(x, z)`, and its least and most over columns `min` to `max` (x, z), inclusive, at any height. */
pub fn of_columns(
    at: impl Fn(i64, i64) -> Fx + Send + Sync + 'static,
    over: impl Fn([i64; 2], [i64; 2]) -> (Fx, Fx) + Send + Sync + 'static,
) -> Scalar {
    Scalar::new(move |[x, _, z]| at(x, z), move |b: &Box3| over([b.min[0], b.min[2]], [b.max[0] - 1, b.max[2] - 1]))
}

/** Scalars read by column (x, z), at any height: what a world on a ground reads of its shares. */
pub trait Columns {
    /** It at column (x, z). */
    fn column(&self, x: i64, z: i64) -> Fx;
}

impl Columns for Scalar {
    fn column(&self, x: i64, z: i64) -> Fx {
        self.at([x, 0, z])
    }
}

/**
 * The shares of `bids` for columns, as worldgen's land shares them,
 * decided at `scale` and kept by column as plan `name`, read dithered
 * where sites are further apart (Plan::dithered).
 */
pub fn shares<const N: usize>(name: &'static str, scale: i64, width: Fx, bids: [Scalar; N]) -> [Scalar; N] {
    land::kept(width, bids, |decide| {
        let shares = Plan::dithered(name, scale, move |x, z| decide([x, 0, z]));
        Arc::new(move |[x, _, z]| shares.at(x, z))
    })
}

/**
 * A pull towards `to` (Pull::pull) from `d` away: all of it at none
 * and inside, easing to none `over` past; none further.
 */
pub fn eased(to: Fx, over: Units, d: Units) -> Option<(Fx, Fx)> {
    (d < over).then(|| (to, Fx::ONE - smoothstep(Fx::ZERO, over.to_fx(), d.to_fx())))
}

/**
 * The lie of the land: the ground's height at each column, with its
 * fraction, made band by band. A band is the one before pulled
 * (`shaped`) by what reads only the bands before it. A function of
 * position, not a plan, so plans of any scale may read it; kept by
 * column, as several read each column. The ground layers stand on is its
 * last band, a plan (`ground`).
 */
#[derive(Clone)]
pub struct Lie(Arc<Band>);

struct Band {
    height: Box<dyn Fn(i64, i64) -> Fx + Send + Sync>,
    kept: Kept<Fx>,
}

impl Lie {
    pub fn new(height: impl Fn(i64, i64) -> Fx + Send + Sync + 'static) -> Lie {
        Lie(Arc::new(Band { height: Box::new(height), kept: Kept::new() }))
    }

    /** The height at column (x, z), with its fraction. */
    pub fn at(&self, x: i64, z: i64) -> Fx {
        let slot = slot(x, z);
        self.0.kept.get(slot, x, z).unwrap_or_else(|| {
            let h = (self.0.height)(x, z);
            self.0.kept.keep(slot, x, z, h);
            h
        })
    }

    /**
     * The next band: this one pulled by `pulls`, each towards its height
     * as hard as it says, the same in whatever order they come. A rule:
     * they add up to all of a column at most, so none undoes another's;
     * broken, it panics.
     */
    pub fn shaped(&self, pulls: Vec<Arc<dyn Pull>>) -> Lie {
        let under = self.clone();
        Lie::new(move |x, z| {
            let ground = under.at(x, z);
            let (mut pulled, mut how_hard) = (ground, Fx::ZERO);
            for (to, w) in pulls.iter().filter_map(|p| p.pull(x, z, ground)) {
                pulled += (to - ground) * w;
                how_hard += w;
            }
            assert!(
                (Fx::ZERO..=Fx::ONE).contains(&how_hard),
                "pulls on column ({x}, {z}) are {} hard, not up to all of it; see \"Ground\" in {DOCS}",
                how_hard.to_f64()
            );
            pulled
        })
    }

    /** The ground layers stand on: this band rounded to units, kept by plan at `scale`, rising at most `slope` a column. */
    pub fn ground(&self, scale: i64, slope: i64) -> Arc<Sloped> {
        let lie = self.clone();
        Arc::new(Sloped::new(Arc::new(Plan::new("ground", scale, move |x, z| lie.at(x, z).round_units().0)), slope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldgen::noise::fx;

    /** Pulls towards `to` with `how_hard` where x is at least `from`. */
    struct Towards {
        from: i64,
        to: Fx,
        how_hard: Fx,
    }

    impl Pull for Towards {
        fn pull(&self, x: i64, _: i64, _: Fx) -> Option<(Fx, Fx)> {
            (x >= self.from).then_some((self.to, self.how_hard))
        }
    }

    fn pull(from: i64, to: f64, how_hard: f64) -> Arc<dyn Pull> {
        Arc::new(Towards { from, to: fx(to), how_hard: fx(how_hard) })
    }

    #[test]
    fn a_band_pulls_as_hard_as_each_says() {
        let flat = Lie::new(|_, _| fx(10.0));
        let half = flat.shaped(vec![pull(0, 20.0, 0.5)]);
        assert_eq!((half.at(-1, 0), half.at(0, 0)), (fx(10.0), fx(15.0)));
        assert_eq!(flat.shaped(vec![pull(0, 20.0, 0.25), pull(0, 30.0, 0.75)]).at(0, 0), fx(27.5));
    }

    #[test]
    #[should_panic(expected = "pulls on column (0, 0) are 1.5 hard")]
    fn refuses_pulls_adding_up_to_more_than_all_of_a_column() {
        Lie::new(|_, _| fx(10.0)).shaped(vec![pull(0, 20.0, 1.0), pull(0, 40.0, 0.5)]).at(0, 0);
    }
}
