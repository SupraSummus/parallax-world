//! The valley's course, and the route its path takes along its floor.
//!
//! The course is the spine the valley winds along, kilometres either way:
//! noise along z, smooth at every scale, so it never runs straight for
//! long, through x = 0 at z = 0, where the camera starts. The ranges
//! stand either side of it, the towns on it, the railway beside it.
//!
//! The path is not drawn about it: it is found on the valley's floor, a
//! curve (worldgen::curve) in pieces along the course, on it at their
//! ends. At each level the point between two neighbours is the lowest of
//! the floor within the level's wander, so the route keeps to hollows and
//! winds round rises; seen from above it turns at its points, so it is
//! smoothed, a B-spline through them.

use std::rc::Rc;

use worldgen::curve::Curve;
use worldgen::domain::Scope;
use worldgen::noise::{Fx, fx, hash, unit};
use worldgen::plan::Plan;
use worldgen::units::{UNIT_SIZE, Units, m};

use crate::path::Spine;

/** The course's octaves: each one's swing across, from its least to its most, and its period along z, metres. */
const OCTAVES: [(f64, f64); 2] = [(1200.0, 8000.0), (220.0, 2200.0)];
/** The most the course runs across z: a quintic ease runs at most 15/8 of its rise over its run. */
pub const COURSE_SLOPE: f64 = 1.875 * (OCTAVES[0].0 / OCTAVES[0].1 + OCTAVES[1].0 / OCTAVES[1].1);

/** The valley's course of `scope`. */
pub fn course(scope: &Scope) -> Spine {
    let seeds = [scope.seed("long"), scope.seed("short")];
    let at = move |zm: f64| -> f64 { OCTAVES.iter().zip(seeds).map(|(&(swing, period), seed)| swing * noise(seed, zm / period)).sum() };
    let origin = at(0.0);
    Spine::new(move |z| Fx::m(at(z as f64 * UNIT_SIZE) - origin))
}

/** Noise along a line: a value in 0..1 at each whole t, eased between them by a quintic, so it bends smoothly. */
fn noise(seed: u64, t: f64) -> f64 {
    let i = t.floor();
    let f = t - i;
    let at = |i: f64| unit(hash(seed, i as i64, 0, 0));
    at(i) + (at(i + 1.0) - at(i)) * f * f * f * (f * (f * 6.0 - 15.0) + 10.0)
}

/** The route's pieces along z, on the course at their ends, each refined LEVELS times to points FINEST apart. */
const PIECE: i64 = m(1024.0);
const LEVELS: usize = 5;
const FINEST: i64 = PIECE >> LEVELS;
/**
 * How far a level's point may stand across from its neighbours' middle:
 * less than half the level before's, so the route winds most over
 * hundreds of metres and gently over tens.
 */
const WANDER: [i64; LEVELS] = [m(64.0), m(24.0), m(9.0), m(3.0), m(1.0)];
/** Places tried either side of the middle, evenly within the wander. */
const TRIES: i64 = 8;
/** Plan scale of the route's pieces: under the towns' (which the path bends onto), over the railway's (which reads the ground about it). */
const SCALE: i64 = m(512.0);

/** The route along the valley's floor: its points, by piece. */
pub struct Route {
    pieces: Plan<Option<Rc<Vec<[Fx; 2]>>>>,
}

impl Route {
    /** The route along `course` keeping lowest on `floor`, the ground at (x, z) before anything shapes it. */
    pub fn new(course: Spine, floor: impl Fn(i64, i64) -> Fx + 'static) -> Route {
        let wander = WANDER.iter().map(|&w| Units(w).to_fx()).collect();
        let rule = Curve::new(wander, move |level, a: [Fx; 2], b: [Fx; 2]| {
            let (middle, z) = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2);
            // Short of the wander by a step, room for rounding the middle.
            let step = Units(WANDER[level]).to_fx() / (TRIES + 1);
            // The lowest, nearest the middle of those as low.
            let tries = (0..=TRIES).flat_map(|j| [j, -j]).skip(1).map(|j| middle + step * j);
            let x = tries.min_by_key(|x| floor(x.round_units().0, z.floor_units().0)).expect("the middle");
            [x, z]
        });
        let end = move |k: i64| [course.x(k * PIECE), Units(k * PIECE).to_fx()];
        Route { pieces: Plan::new("route", SCALE, move |_, k| Some(Rc::new(rule.points([end(k), end(k + 1)])))) }
    }

    fn piece(&self, k: i64) -> Rc<Vec<[Fx; 2]>> {
        self.pieces.at(0, k).expect("a piece")
    }

    /** Its x at z, with its fraction: a uniform cubic B-spline through its points. */
    pub fn x(&self, z: i64) -> Fx {
        let (n, t) = (1 << LEVELS, z.rem_euclid(FINEST) as f64 / FINEST as f64);
        let (k, i) = (z.div_euclid(PIECE), z.rem_euclid(PIECE) / FINEST);
        // Point i - 1 to i + 2 of piece k: its own, which end on its neighbours' first, but at its ends.
        let piece = self.piece(k);
        let point = |i: i64| match i {
            -1 => self.piece(k - 1)[n - 1][0],
            i if i > n as i64 => self.piece(k + 1)[1][0],
            i => piece[i as usize][0],
        };
        let [p0, p1, p2, p3] = [i - 1, i, i + 1, i + 2].map(|i| point(i).to_f64());
        let u = 1.0 - t;
        let (t2, t3) = (t * t, t * t * t);
        fx((u * u * u * p0 + (3.0 * t3 - 6.0 * t2 + 4.0) * p1 + (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) * p2 + t3 * p3) / 6.0)
    }
}
