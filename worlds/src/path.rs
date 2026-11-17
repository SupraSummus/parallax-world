//! A path along z: its centre line at each z, which what is laid out
//! about the path measures across, and a curve (worldgen::curve) through
//! it, which what is on the path or keeps off it measures from.

use std::rc::Rc;

use worldgen::curve::Curve;
use worldgen::noise::{Fx, fx};
use worldgen::plan::Plan;
use worldgen::units::{Units, m};

use crate::forest::PATH_SCALE;

/** The curve's pieces along z, each refined to points a metre apart. */
const PIECE: i64 = m(8.0);
const LEVELS: usize = 3;
/**
 * The steepest a winding route's centre line runs across z, with room:
 * from its bends (forest::Relief::path_x), at most 0.68, and its ease
 * onto a settlement's main street from up to 14 m off, at most 0.36 more.
 * A path may run steeper, as it says.
 */
pub const SLOPE: f64 = 1.125;
/**
 * How fast its slope changes (per unit), with room: from the forest's
 * route's bends at most 0.0049, and its ease onto a main street at most
 * 0.0071 more; the valley's route and its ease, less. The curve checks it
 * by its wanders, and `Path::new` the slope, where queries look.
 */
const BEND: Fx = fx(0.0135);
/** Over how far the curve passes from the centre line's x at any z, between points a metre apart: BEND·1 m²/8. */
const SAG: Fx = fx(0.125);

pub struct Path {
    centre: Rc<Plan<Fx>>,
    curve: Curve<2>,
    /** Over √(1 + slope²): the path keeps further than a distance from a column this many times as far across. */
    across: Fx,
}

impl Path {
    /** The path through x(z) at each z, its centre line, with its fraction, running at most `slope` across z. */
    pub fn new(x: impl Fn(i64) -> Fx + 'static, slope: f64) -> Path {
        let across = fx((1.0 + slope * slope).sqrt() + 0.01);
        let slope = fx(slope);
        let centre = Rc::new(Plan::new("path", PATH_SCALE, move |_, z| x(z)));
        // A point halves a run h long along z: it is off its ends' middle by at most BEND·h²/8.
        let wander = (0..LEVELS).map(|level| BEND * Units((PIECE >> level).pow(2)) / 8).collect();
        let curve = {
            let centre = centre.clone();
            Curve::new(wander, move |_, a: [Fx; 2], b: [Fx; 2]| {
                let z = ((a[1] + b[1]) / 2).floor_units().0;
                let mid = [centre.at(0, z), Units(z).to_fx()];
                for [from, to] in [[a, mid], [mid, b]] {
                    assert!(
                        (to[0] - from[0]).abs() <= slope * (to[1] - from[1]),
                        "the path runs steeper than its slope (path.rs) at z = {z}"
                    );
                }
                mid
            })
        };
        Path { centre, curve, across }
    }

    /** The centre line at z, with its fraction. */
    pub fn x(&self, z: i64) -> Fx {
        self.centre.at(0, z)
    }

    /** How far column (x, z) is across the valley from the centre line, along x, with its fraction. */
    pub fn across(&self, x: i64, z: i64) -> Fx {
        (Units(x).to_fx() - self.x(z)).abs()
    }

    /** Whether column (x, z) is nearer the path than `distance`. */
    pub fn by(&self, x: i64, z: i64, distance: Fx) -> bool {
        // Most columns are decided by how far across they are: the curve only where it could be either.
        let across = self.across(x, z);
        if across >= distance * self.across + SAG {
            return false;
        }
        if across + SAG < distance {
            return true;
        }
        let p = [Units(x).to_fx(), Units(z).to_fx()];
        let r = distance.floor_units().0 + 1;
        ((z - r).div_euclid(PIECE)..=(z + r).div_euclid(PIECE)).any(|k| {
            let ends = [k * PIECE, (k + 1) * PIECE].map(|z| [self.x(z), Units(z).to_fx()]);
            self.curve.near(ends, p, distance).is_some()
        })
    }
}
