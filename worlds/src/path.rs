//! The forest's path, along the valley: its centre line at each z, which
//! the valley is laid out across, and a curve (worldgen::curve) through
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
 * The steepest the centre line runs across z, and how fast that changes
 * (per unit), with room: from the route's bends (forest::Relief::path_x),
 * at most 0.68 and 0.0049, and its ease onto a town's main street from
 * up to 14 m off, at most 0.36 and 0.0071 more. The curve checks BEND
 * by its wanders, and `Path::new` SLOPE, where queries look.
 */
const SLOPE: Fx = fx(1.125);
const BEND: Fx = fx(0.0135);
/** Over √(1 + SLOPE²): the path keeps further than a distance from a column this many times as far across. */
const ACROSS: Fx = fx(1.51);
/** Over how far the curve passes from the centre line's x at any z, between points a metre apart: BEND·1 m²/8. */
const SAG: Fx = fx(0.125);

pub struct Path {
    centre: Rc<Plan<Fx>>,
    curve: Curve<2>,
}

impl Path {
    /** The path through x(z) at each z, its centre line, with its fraction. */
    pub fn new(x: impl Fn(i64) -> Fx + 'static) -> Path {
        let centre = Rc::new(Plan::new("path", PATH_SCALE, move |_, z| x(z)));
        // A point halves a run h long along z: it is off its ends' middle by at most BEND·h²/8.
        let wander = (0..LEVELS).map(|level| BEND * Units((PIECE >> level).pow(2)) / 8).collect();
        let curve = {
            let centre = centre.clone();
            Curve::new(wander, move |_, a: [Fx; 2], b: [Fx; 2]| {
                let z = ((a[1] + b[1]) / 2).floor_units().0;
                let mid = [centre.at(0, z), Units(z).to_fx()];
                for [from, to] in [[a, mid], [mid, b]] {
                    assert!((to[0] - from[0]).abs() <= SLOPE * (to[1] - from[1]), "the path runs steeper than SLOPE (path.rs) at z = {z}");
                }
                mid
            })
        };
        Path { centre, curve }
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
        if across >= distance * ACROSS + SAG {
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
