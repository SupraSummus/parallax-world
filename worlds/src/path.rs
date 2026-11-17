//! Spines: lines along z, which what is laid out about them measures
//! across; and paths: a spine with a curve (worldgen::curve) through it,
//! which what is on the path or keeps off it measures from.

use std::rc::Rc;

use worldgen::curve::Curve;
use worldgen::noise::{Fx, fx};
use worldgen::plan::Plan;
use worldgen::units::{Units, m};

use crate::forest::PATH_SCALE;

/**
 * A line along z, by its x at each z: a valley's course, a path's centre.
 * A function of position, so a plan of any band may read it.
 */
#[derive(Clone)]
pub struct Spine(Rc<dyn Fn(i64) -> Fx>);

impl Spine {
    pub fn new(x: impl Fn(i64) -> Fx + 'static) -> Spine {
        Spine(Rc::new(x))
    }

    /** Its x at z, with its fraction. */
    pub fn x(&self, z: i64) -> Fx {
        (self.0)(z)
    }

    /** How far column (x, z) is across it, along x (east of it more than 0), with its fraction. */
    pub fn across(&self, x: i64, z: i64) -> Fx {
        Units(x).to_fx() - self.x(z)
    }
}

/** The curve's pieces along z, each refined to points a metre apart. */
const PIECE: i64 = m(8.0);
const LEVELS: usize = 3;
/**
 * How fast the centre line's slope changes (per unit), with room: from the
 * forest's route's bends at most 0.0049, and its ease onto a main street
 * at most 0.0071 more; the valley's route and its ease, less. The curve
 * checks it by its wanders, where queries look.
 */
const BEND: Fx = fx(0.0135);
/** Over how far the curve passes from the centre line's x at any z, between points a metre apart: BEND·1 m²/8. */
const SAG: Fx = fx(0.125);

pub struct Path {
    centre: Rc<Plan<Fx>>,
    curve: Curve<2>,
    /** By piece, √(1 + s²) for its steepest segment's slope s: the path keeps further than a distance from a column this many times as far across. */
    stretch: Plan<Fx>,
}

impl Path {
    /** The path along `spine`, its centre line. */
    pub fn new(spine: Spine) -> Path {
        let centre = Rc::new(Plan::new("path", PATH_SCALE, move |_, z| spine.x(z)));
        // A point halves a run h long along z: it is off its ends' middle by at most BEND·h²/8.
        let wander = (0..LEVELS).map(|level| BEND * Units((PIECE >> level).pow(2)) / 8).collect();
        let curve = {
            let centre = centre.clone();
            Curve::new(wander, move |_, a: [Fx; 2], b: [Fx; 2]| {
                let z = ((a[1] + b[1]) / 2).floor_units().0;
                [centre.at(0, z), Units(z).to_fx()]
            })
        };
        let stretch = {
            let centre = centre.clone();
            Plan::new("path's stretch", PATH_SCALE, move |_, k| {
                // The curve's segments, a metre long: its points are the centre line's.
                let x: Vec<f64> = (0..=PIECE / m(1.0)).map(|i| centre.at(0, k * PIECE + i * m(1.0)).to_f64()).collect();
                let steepest = x.windows(2).map(|w| (w[1] - w[0]).abs() / m(1.0) as f64).fold(0.0, f64::max);
                // Rounded up.
                fx((1.0 + steepest * steepest).sqrt() + 1e-4)
            })
        };
        Path { centre, curve, stretch }
    }

    /** The centre line at z, with its fraction. */
    pub fn x(&self, z: i64) -> Fx {
        self.centre.at(0, z)
    }

    /** `Spine::across`, from the centre line kept. */
    pub fn across(&self, x: i64, z: i64) -> Fx {
        Units(x).to_fx() - self.x(z)
    }

    /** Whether column (x, z) is nearer the path than `distance`. */
    pub fn by(&self, x: i64, z: i64, distance: Fx) -> bool {
        let r = distance.floor_units().0 + 1;
        let pieces = || (z - r).div_euclid(PIECE)..=(z + r).div_euclid(PIECE);
        // Most columns are decided by how far across they are: the curve only where it could be either.
        let across = self.across(x, z).abs();
        if across + SAG < distance {
            return true;
        }
        let stretch = pieces().map(|k| self.stretch.at(0, k)).max().expect("a piece");
        if across >= distance * stretch + SAG {
            return false;
        }
        let p = [Units(x).to_fx(), Units(z).to_fx()];
        pieces().any(|k| {
            let ends = [k * PIECE, (k + 1) * PIECE].map(|z| [self.x(z), Units(z).to_fx()]);
            self.curve.near(ends, p, distance).is_some()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * Round a spine as steep as 3 in 4 and bending as fast as BEND lets
     * it, `by` says what the distance to its curve, the polyline through
     * its points a metre apart, says: its shortcuts across never err.
     */
    #[test]
    fn by_agrees_with_the_curve() {
        let x = |z: i64| Fx::m(6.0 * (z as f64 / m(8.0) as f64).sin());
        let path = Path::new(Spine::new(x));
        let point = |i: i64| [x(i * m(1.0)).to_f64(), (i * m(1.0)) as f64];
        let to_segment = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
            let (d, e) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
            let t = ((d[0] * e[0] + d[1] * e[1]) / (d[0] * d[0] + d[1] * d[1])).clamp(0.0, 1.0);
            (e[0] - t * d[0]).hypot(e[1] - t * d[1])
        };
        for distance in [Fx::m(1.1), Fx::m(2.8)] {
            for (x, z) in (-m(12.0)..m(12.0)).flat_map(|x| (0..m(40.0)).map(move |z| (x, z))) {
                let p = [x as f64, z as f64];
                let i = z.div_euclid(m(1.0));
                let nearest = (i - 4..=i + 4).map(|i| to_segment(p, point(i), point(i + 1))).fold(f64::MAX, f64::min);
                // Rounding apart, at the distance itself.
                if (nearest - distance.to_f64()).abs() > 1e-3 {
                    assert_eq!(path.by(x, z, distance), nearest < distance.to_f64(), "({x}, {z}) is {nearest} from the curve");
                }
            }
        }
    }
}
