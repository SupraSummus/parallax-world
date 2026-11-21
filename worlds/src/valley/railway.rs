//! The railway: a main line along the valley, on one side of it, past
//! the towns and their farmland, with a station by each town it passes on
//! the ground. A domain of the valley (valley.rs) that reads only the
//! valley's ground before it shapes it (`valley::Terrain`): it bids for
//! the land along its line, cuts and fills the ground to its track, and
//! draws its track, bridges, tunnels and stations. See "Railway" in
//! docs/generators.md.
//!
//! The line is a curve (worldgen::curve) in pieces along the valley, its
//! points (x, y, z), x across from the valley's course (course.rs), so it
//! winds with it, within a band clear of the towns and up the mountains'
//! faces: at each level a point between two neighbours, the place across
//! within its wander and the height within the grade where the track
//! between them costs least over the ground under it: earth cut or filled
//! by its cross-section, a bridge or a tunnel much more. So it bends along
//! a slope and climbs or falls with the ground to keep off deep earth, and
//! off tunnels and bridges most, where its bends and grade let it. Seen from above that line turns
//! at its points, so its x is averaged over SMOOTH_HALF points either
//! side, across the pieces' ends, and it bends gently. Where the ground is
//! far from the track the line tunnels or bridges, nearer it cuts or
//! fills, by the same comparison at each point, and no run of bridge or
//! tunnel is shorter than MIN_RUN points.

use std::sync::Arc;

use worldgen::column::Column;
use worldgen::curve::{self, Curve, Near};
use worldgen::domain::{Domain, Scope};
use worldgen::frame::{Frame, Part, facing};
use worldgen::generator::{Field, Kind, Layer, Node, Op, Population, Span};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, fx, hash, hash64, smoothstep};
use worldgen::plan::{self, Holding, Large, Plan, Sloped};
use worldgen::units::{Units, m};

use super::Terrain;
use super::course::COURSE_SLOPE;
use super::moor;
use super::mountains;
use super::town::{self, House, K as T, Style, Town, Towns, thing};
use crate::forest::K as F;

kinds! {
    base: moor::BASE + moor::KINDS.len() as u8;
    ballast: 3, ["#8a847b", "#7b766e"];
}

/** The line's pieces along z, each refined LEVELS times, to points FINEST apart. */
const PIECE: i64 = m(4096.0);
const LEVELS: usize = 9;
const FINEST: i64 = PIECE >> LEVELS;
/**
 * How far a level's point may stand across from its neighbours' middle,
 * the first levels' (half the level before's, so each turns the line by
 * as much at its points); the finer stay at the middle. Their points are
 * PIECE >> WANDER.len() apart at the closest, further than smoothing's
 * window, so a window holds one at most.
 */
const WANDER: [i64; 4] = [m(96.0), m(48.0), m(24.0), m(12.0)];
/** The line's x is the mean over this many points either side, which spreads the turn at each point that wanders. */
const SMOOTH_HALF: i64 = 12;
const _: () = assert!((2 * SMOOTH_HALF + 1) * FINEST < PIECE >> WANDER.len());
/** The fewest points a bridge or a tunnel is carried over (32 m): shorter, the ground is cut or filled. */
const MIN_RUN: i64 = 4;
/** Places tried either side of the middle, evenly within the wander. */
const TRIES: i64 = 6;
/**
 * The band across from the course the line keeps to, on its side: from
 * INNER, past the furthest a town and the ground the railway shapes reach
 * (by construction, as far along the town as the course runs across), to
 * OUTER, up the mountains' faces.
 */
const INNER: i64 = town::ACROSS + m(4.0) + (COURSE_SLOPE * town::LONG as f64) as i64 + REACH + 1;
const OUTER: i64 = INNER + m(256.0);
/**
 * The steepest the line runs across from the course: its ends across the
 * band over a piece, and each level that wanders by its wander over half
 * its segment. With the course's, under 1 in 1, so a column further than
 * 1½ times a distance across from it is further.
 */
const SLANT: f64 = {
    let (mut slant, mut level) = ((OUTER - INNER) as f64 / PIECE as f64, 0);
    while level < WANDER.len() {
        slant += WANDER[level] as f64 / (PIECE >> (level + 1)) as f64;
        level += 1;
    }
    slant
};
const _: () = assert!(SLANT + COURSE_SLOPE < 1.0);
/**
 * The most it turns from the course at a point, radians: where a level
 * wanders (or at a piece's end), the segments either side differ in slope
 * by twice the slant at most, spread over smoothing's window. So it bends
 * round no tighter than FINEST / TURN (400 m), the course's bends added.
 */
#[cfg(test)]
const TURN: f64 = 2.0 * SLANT / (2 * SMOOTH_HALF + 1) as f64;
/** The steepest it climbs: 4 in 256 (1.6 %), a whole number of Fx's least steps over half a segment of any level. */
const GRADE: f64 = 4.0 / 256.0;
/** The lowest its pieces' ends lie; the highest, a grade's rise over a piece above. */
const LOW: Fx = Fx::m(-6.0);
/** Samples of the ground over each half of a segment, at most; at a piece's end, along z either side, ENDS_APART apart. */
const SAMPLES: i64 = 8;
const ENDS_APART: i64 = m(64.0);
/** Heights a fit tries either side of the middle of its range; places an end tries across the band, ACROSS_STEP apart. */
const HEIGHTS: i64 = 8;
const ACROSS_STEP: i64 = m(16.0);
/** What keeping straight is worth: a metre of earth a sample for each STRAIGHT metres a point stands across from its neighbours' middle. */
const STRAIGHT: i64 = 32;
/**
 * Ground further above the track than CUT metres is tunnelled, further
 * below than FILL bridged; nearer, cut or filled, its sides SLOPE (rise
 * over run), so earth's cross-section is the formation's width times its
 * depth and SPREAD more.
 */
const CUT: f64 = 10.0;
const FILL: f64 = 8.0;
const SLOPE: f64 = 2.0 / 3.0;
const SPREAD: i64 = (2.0 * FORMATION as f64 * SLOPE) as i64;
/** What a tunnel or a bridge costs a sample at the least: a few times the deepest earth. */
const COSTLY: Fx = Fx::m(100.0);
/** Half the formation's width; at a station EXTRA more, for its platform and building, easing back over EASE past its ends. */
const FORMATION: i64 = m(3.0);
const EXTRA: i64 = m(14.0);
const STATION: i64 = m(50.0);
const EASE: i64 = m(30.0);
/** How far a cutting's sides reach past the formation, and a fill's past a bridge's ends. */
const SIDES: i64 = m(CUT.max(FILL) / SLOPE);
/** How far round the line it shapes the ground at most: the formation, a station's room, and a cutting's sides. */
const REACH: i64 = FORMATION + EXTRA + SIDES;
/** Plan scale of the line's pieces: under the towns' (the ground it reads is levelled under them), over the landmarks' and steadings' (which keep off it). */
const BAND: i64 = m(256.0);
/** Plan scale of what it is at each column. */
const SCALE: i64 = m(4.0);

// The track and what carries it, in units, across from the line.
const BALLAST: i64 = m(1.75);
const SLEEPERS: i64 = m(1.25);
/** Half the gauge (1.435 m), to the middle of a rail a unit wide. */
const RAIL: Fx = fx(5.74);
/** The rails' height over the ballast. */
const RAILS_HIGH: i64 = 3;
/** Sleepers every SLEEPER along the line, SLEEPER_THICK of it; piers every PIER, PIER_THICK thick. */
const SLEEPER: i64 = 5;
const SLEEPER_THICK: i64 = 2;
/** What covers the sleepers' strip seen from above: the rails, 2 units of its 20, and the sleepers of the rest. */
const IRON: f64 = 0.1;
const COVER: f64 = IRON + (1.0 - IRON) * SLEEPER_THICK as f64 / SLEEPER as f64;
const PIER: i64 = m(20.0);
const PIER_THICK: i64 = m(1.5);
const PIERS: i64 = m(2.0);
const DECK: i64 = m(2.5);
const BORE: i64 = m(2.5);
const WALLS: i64 = m(2.5);
const LINING: i64 = m(0.5);
/** The bores' order: after every other layer of the valley. */
const BORES: f64 = 9.0;
/** A platform from PLATFORM to its building, PLATFORM_HIGH over the track; the building behind it, on the formation. */
const PLATFORM: i64 = m(2.25);
const PLATFORM_HIGH: i64 = 6;
const BUILDING: [i64; 2] = [m(14.0), m(7.0)];
const BUILDING_FROM: i64 = m(8.0);
/** Rows along z the towns a piece passes are looked for at (town.rs: towns are 900 m apart at least, a row's within 600 m). */
const TOWN_ROWS: i64 = m(400.0);
/** The stations' cell, and how far a building reaches past it. */
const CELL: i64 = m(16.0);
const STATION_REACH: [i64; 3] = [m(10.0), m(16.0), m(10.0)];

/** What carries the track where the ground is: compared at the line, the same across it. */
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Structure {
    /** The ground cut or filled to the track. */
    #[default]
    Earth,
    Bridge,
    Tunnel,
}

/** What the ground is to a track `top` high: tunnelled, bridged or cut and filled. */
fn structure(ground: Fx, top: Fx) -> Structure {
    if ground - top > Fx::m(CUT) {
        Structure::Tunnel
    } else if top - ground > Fx::m(FILL) {
        Structure::Bridge
    } else {
        Structure::Earth
    }
}

/**
 * A piece of the line, made once: its points (x, y, z), FINEST apart
 * along z, what carries the track at each, and the towns it may have
 * stations by.
 */
struct Piece {
    points: Vec<[Fx; 3]>,
    structure: Vec<Structure>,
    towns: Vec<Town>,
    /** Its points seen from above, x and z. */
    xz: Vec<[Fx; 2]>,
}

impl Piece {
    /** Where the track is `along` (finest segments, with the fraction) the piece: its x, its top, and what carries it, as at the nearer point. */
    fn at(&self, along: Fx) -> (Fx, Fx, Structure) {
        let i = (along.floor_units().0 as usize).min(self.points.len() - 2);
        let t = along - Units(i as i64).to_fx();
        let lerp = |a: Fx, b: Fx| a + (b - a) * t;
        let (p, q) = (self.points[i], self.points[i + 1]);
        (lerp(p[0], q[0]), lerp(p[1], q[1]), self.structure[i + usize::from(t >= Fx::HALF)])
    }
}

/** What carries the track at each point, from the `ground` under it and its `top`: a bridge or tunnel fewer than MIN_RUN points long is cut or filled instead. */
fn structures(ground: &[Fx], top: &[Fx]) -> Vec<Structure> {
    let s: Vec<Structure> = ground.iter().zip(top).map(|(&g, &t)| structure(g, t)).collect();
    s.chunk_by(|a, b| a == b)
        .flat_map(|run| std::iter::repeat_n(if run.len() < MIN_RUN as usize { Structure::Earth } else { run[0] }, run.len()))
        .collect()
}

/** Where the line's pieces end: from position alone. */
struct Ends {
    /** Where piece 0 starts along z, by the seed: no piece ends where tiles meet. */
    phase: i64,
    /** The side of the valley it runs on. */
    side: i64,
    terrain: Arc<Terrain>,
}

impl Ends {
    fn z(&self, k: i64) -> i64 {
        self.phase + k * PIECE
    }

    /** The piece z is in. */
    fn piece(&self, z: i64) -> i64 {
        (z - self.phase).div_euclid(PIECE)
    }

    /** Piece k's point at z or the last before it, if z is on the piece; else its first or last. */
    fn point(&self, k: i64, z: i64) -> usize {
        ((z - self.z(k)).clamp(0, PIECE) / FINEST) as usize
    }

    /** Across from the course `across` (the side's sign aside) kept to the band. */
    fn band(&self, across: Fx) -> Fx {
        (across * self.side).clamp(Units(INNER).to_fx(), Units(OUTER).to_fx()) * self.side
    }

    /**
     * End k, across from the course: the place across the band, and the
     * height a piece's grade can climb from LOW, where a level track along
     * z costs least, the innermost of those as cheap.
     */
    fn at(&self, k: i64) -> [Fx; 3] {
        let z = self.z(k);
        let places = (INNER..=OUTER).step_by(ACROSS_STEP as usize).map(|a| Units(self.side * a).to_fx());
        let fits = places.map(|x| {
            let samples: Vec<Sample> = (-SAMPLES..=SAMPLES)
                .map(|j| Sample { ground: ground(&self.terrain, x, Units(z + j * ENDS_APART).to_fx()), base: Fx::ZERO, weight: Fx::ONE })
                .collect();
            let (cost, y) = fit(&samples, LOW, LOW + fx(GRADE) * Units(PIECE));
            (cost, [x, y, Units(z).to_fx()])
        });
        fits.min_by_key(|(cost, _)| *cost).expect("a place").1
    }
}

/** The ground `across` from the course at z. */
fn ground(terrain: &Terrain, across: Fx, z: Fx) -> Fx {
    let z = z.floor_units().0;
    terrain.ground((across + terrain.course.x(z)).round_units().0, z)
}

/** Point p, across from the course, in the world. */
fn turned(terrain: &Terrain, p: [Fx; 3]) -> [Fx; 3] {
    [p[0] + terrain.course.x(p[2].floor_units().0), p[1], p[2]]
}

/**
 * What a track `top` high costs over ground, per sample, in metres of
 * earth over the formation's width: cut or filled, its cross-section,
 * which grows with the square of its depth as its sides slope out;
 * tunnelled or bridged, COSTLY, and the more the deeper or higher. So
 * the line keeps to shallow earth where its bends and grade let it, and
 * off tunnels and bridges most.
 */
fn cost(ground: Fx, top: Fx) -> Fx {
    let d = (ground - top).abs();
    if structure(ground, top) == Structure::Earth { d + d * d / Units(SPREAD) } else { COSTLY + d }
}

/** The ground under a track, where its top is `base` and `weight` times the height being fitted. */
struct Sample {
    ground: Fx,
    base: Fx,
    weight: Fx,
}

/** The height from `low` to `high` that costs least over `samples`, nearest the middle of those as cheap; and its cost. */
fn fit(samples: &[Sample], low: Fx, high: Fx) -> (Fx, Fx) {
    let (middle, step) = ((low + high) / 2, (high - low) / (2 * HEIGHTS));
    let heights = (0..=HEIGHTS).flat_map(|j| [j, -j]).skip(1).map(|j| middle + step * j);
    let costs = heights.map(|y| (samples.iter().map(|s| cost(s.ground, s.base + s.weight * y)).sum::<Fx>(), y));
    costs.min_by_key(|(cost, _)| *cost).expect("the middle")
}

/**
 * The rule the line is made by: between neighbours a and b of `level`,
 * the place within the level's wander across (and the band) and the
 * height within the grade from both, so no segment of any level climbs
 * steeper, where the track from a to b through it costs least over the
 * ground under it, sampled; with what it stands across from their
 * middle, so it keeps straight where turning gains little.
 */
fn refine(ends: &Ends, level: usize, a: [Fx; 3], b: [Fx; 3]) -> [Fx; 3] {
    let z = (a[2] + b[2]) / 2;
    // A grade's rise over half a segment of the level.
    let rise = fx(GRADE) * Units(PIECE >> (level + 1));
    let (low, high) = (a[1].max(b[1]) - rise, a[1].min(b[1]) + rise);
    let middle = (a[0] + b[0]) / 2;
    let places: Vec<Fx> = match WANDER.get(level) {
        // Within the band: so is the middle, between two points in it, so no further from it.
        Some(&wander) => {
            let step = Units(wander).to_fx() / TRIES;
            (0..=TRIES).flat_map(|j| [j, -j]).skip(1).map(|j| ends.band(middle + step * j)).collect()
        }
        None => vec![middle],
    };
    // No more than a sample a point of the finest level.
    let n = SAMPLES.min((PIECE >> (level + 1)) / FINEST);
    let fits = places.into_iter().map(|x| {
        // Along each half, its track from the neighbour's height to the one fitted, from the point itself: what a coarse
        // point stands on, the finer can't climb out of at the grade.
        let samples: Vec<Sample> = [a, b]
            .iter()
            .flat_map(|e| {
                (0..n).map(move |j| {
                    let t = fx(j as f64 / n as f64);
                    let to = Fx::ONE - t;
                    Sample { ground: ground(&ends.terrain, e[0] + (x - e[0]) * to, e[2] + (z - e[2]) * to), base: e[1] * t, weight: to }
                })
            })
            .collect();
        let (cost, y) = fit(&samples, low, high);
        let bent = (x - middle).abs() * (2 * n) / STRAIGHT;
        (cost + bent, [x, y, z])
    });
    fits.min_by_key(|(cost, _)| *cost).expect("the middle").1
}

/** What a column is to the line: kept per column, as the ground, the track and the bid all ask. */
#[derive(Clone, Copy, Default)]
struct At {
    /** How far it is from the line, across, with its fraction. */
    distance: Fx,
    /** How far along the line, in lattice units from the start of its piece 0, which the seed places. */
    along: i64,
    /** The track's top, in units: the formation's under the ballast. */
    top: i64,
    structure: Structure,
    /** How far along the line it is from where the ground carries the track: zero there, none further along a bridge or a tunnel than SIDES. */
    beyond: Option<Fx>,
    /** Half the formation's width there, wider at a station. */
    half: i64,
    /** By a station, on its side: its platform and building's yard. */
    platform: bool,
}

/** A station: by a town, on the line at the town's z. */
#[derive(Clone, Copy, Default)]
struct Station {
    /** Where the line passes it, x and z, and the track's top there. */
    at: [i64; 2],
    top: i64,
    /** The side of the line towards the valley's middle (±1 across x). */
    inner: i64,
    seed: u64,
}

impl Station {
    /** Its building's centre, on the platform. */
    fn centre(&self) -> [i64; 3] {
        self.frame().site([BUILDING[0] / 2, 0, BUILDING[1] / 2])
    }

    /** Its building's frame: u along the platform, v up from it, w back from the track. */
    fn frame(&self) -> Frame {
        let across = self.at[0] + self.inner * (BUILDING_FROM + BUILDING[1] / 2);
        Frame::centred([across, self.top + 1, self.at[1]], [BUILDING[0], 0, BUILDING[1]], facing(-self.inner, 0))
    }
}

/** The line and its stations, from position alone. */
struct Line {
    ends: Arc<Ends>,
    pieces: Large<Piece>,
    towns: Arc<Towns>,
    /** Its stations' seed, and its surfaces'. */
    stations: u64,
    surfaces: u64,
}

impl Line {
    fn piece(&self, k: i64) -> &Piece {
        self.pieces.at(0, k)
    }

    /**
     * Whether column (x, z) may be within `r` of the line: across from
     * the course, the line keeps to its band, and runs across z less than
     * 1 in 1 (SLANT), so a column further across from the band than 1½ r
     * (and a unit for rounding each) is further than r from it.
     */
    fn may_reach(&self, x: i64, z: i64, r: i64) -> bool {
        let across = self.ends.side * self.ends.terrain.course.across(x, z).round_units().0;
        let most = r + r / 2 + 2;
        INNER - most < across && across < OUTER + most
    }

    /**
     * Where the line passes nearest column (x, z), seen from above, if
     * near enough to shape it, and `margin` more: its formation and a
     * cutting's sides, and a station's room by a town. Its piece, and
     * where on it.
     */
    fn near(&self, x: i64, z: i64, margin: i64) -> Option<(i64, &Piece, Near<2>)> {
        let most = REACH + margin;
        if !self.may_reach(x, z, most) {
            return None;
        }
        let p = [Units(x).to_fx(), Units(z).to_fx()];
        (self.ends.piece(z - most)..=self.ends.piece(z + most))
            .filter_map(|k| {
                let piece = self.piece(k);
                let station = piece.towns.iter().any(|t| (z - t.z).abs() < STATION + EASE + REACH);
                let r = if station { most } else { most - EXTRA };
                // Only segments within r along z can be within r.
                let segments = self.ends.point(k, z - r)..self.ends.point(k, z + r + FINEST).min(piece.xz.len() - 1);
                let near = curve::nearest(&piece.xz, segments, p, Units(r).to_fx())?;
                Some((k, piece, near))
            })
            .min_by_key(|(_, _, n)| n.distance)
    }

    /** What column (x, z) is to the line, if within its reach. */
    fn at(&self, x: i64, z: i64) -> Option<At> {
        let (k, piece, near) = self.near(x, z, 0)?;
        let (_, top, structure) = piece.at(near.along);
        let line_z = near.point[1].floor_units().0;
        let inner = (Units(x).to_fx() - near.point[0]).signum() == -self.ends.side;
        // A station's room on the side of the valley's middle.
        let station = piece.towns.iter().find(|t| inner && (line_z - t.z).abs() < STATION + EASE).and_then(|t| self.station(t));
        let (half, platform) = station.map_or((FORMATION, false), |s| {
            let off = (line_z - s.at[1]).abs();
            let wide = Fx::ONE - smoothstep(Units(STATION).to_fx(), Units(STATION + EASE).to_fx(), Units(off).to_fx());
            (FORMATION + (wide * Units(EXTRA)).round_units().0, off < STATION)
        });
        let beyond = match structure {
            Structure::Earth => Some(Fx::ZERO),
            // Easing out past a bridge's ends and into a tunnel's hill: cut off at a portal, the ground stepped by metres.
            Structure::Bridge | Structure::Tunnel => self.to_earth(k, near.along),
        };
        Some(At {
            distance: near.distance,
            along: k * PIECE + (near.along * Units(FINEST)).floor_units().0,
            top: top.round_units().0,
            structure,
            beyond,
            half,
            platform,
        })
    }

    /**
     * How far `along` piece k (segments) is from the nearest point that
     * the ground carries the track at, within SIDES: from halfway to it,
     * where `Piece::at` changes, across the pieces' ends.
     */
    fn to_earth(&self, k: i64, along: Fx) -> Option<Fx> {
        let n = 1i64 << LEVELS;
        let earth = |i: i64| self.piece(k + i.div_euclid(n)).structure[i.rem_euclid(n) as usize] == Structure::Earth;
        let (p, most) = ((along + Fx::HALF).floor_units().0, SIDES / FINEST + 1);
        let ahead = (p..=p + most).find(|&i| earth(i)).map(|i| Units(i).to_fx() - Fx::HALF - along);
        let behind = (p - most..=p).rev().find(|&i| earth(i)).map(|i| along - Units(i).to_fx() - Fx::HALF);
        [ahead, behind].into_iter().flatten().min().map(|d| d * Units(FINEST)).filter(|&d| d < Units(SIDES).to_fx())
    }

    /** The track at z: where it is, x and z, its top, and what carries it. */
    fn track(&self, z: i64) -> ([i64; 2], i64, Structure) {
        let k = self.ends.piece(z);
        let along = Units(z - self.ends.z(k)).to_fx() / Units(FINEST);
        let (x, top, structure) = self.piece(k).at(along);
        ([x.round_units().0, z], top.round_units().0, structure)
    }

    /** The station by `town`, if the line passes it on the ground all along its platform and the ease past it. */
    fn station(&self, town: &Town) -> Option<Station> {
        let (from, to) = (town.z - STATION - EASE, town.z + STATION + EASE);
        // Its points there, a piece at a time: between two on the ground, so is the track.
        let on_ground = (self.ends.piece(from)..=self.ends.piece(to))
            .all(|k| self.piece(k).structure[self.ends.point(k, from)..=self.ends.point(k, to)].iter().all(|&s| s == Structure::Earth));
        if !on_ground {
            return None;
        }
        let (at, top, _) = self.track(town.z);
        Some(Station { at, top, inner: -self.ends.side, seed: hash64(self.stations, town.x, town.z, 0) })
    }

    /** The station whose building's cell column (x, z) is in, if any. */
    fn station_near(&self, x: i64, z: i64) -> Option<Station> {
        let s = self.station(&self.towns.near(z)?)?;
        let [cx, _, cz] = s.centre();
        ((x - cx).abs() <= CELL && (z - cz).abs() <= CELL).then_some(s)
    }
}

/** The valley's railway: its line and stations, and what each column is to them, kept. */
pub struct Railway {
    line: Arc<Line>,
    at: Plan<Option<At>>,
}

impl Railway {
    pub fn new(scope: &Scope, terrain: Arc<Terrain>) -> Railway {
        let s = scope.seed("ends");
        let ends = Arc::new(Ends {
            phase: (hash64(s, 0, 0, 0) % PIECE as u64) as i64,
            side: if hash64(s, 1, 0, 0) & 1 == 0 { 1 } else { -1 },
            terrain,
        });
        let rise = |level: usize| GRADE * (PIECE >> (level + 1)) as f64;
        let across = |level: usize| WANDER.get(level).map_or(0.0, |&w| w as f64);
        // Across and up, and a little for rounding a middle: the rule keeps within each.
        let wander = (0..LEVELS).map(|l| fx(across(l).hypot(rise(l)) + 0.001)).collect();
        let rule = {
            let ends = ends.clone();
            Curve::new(wander, move |level, a, b| refine(&ends, level, a, b))
        };
        let raw = {
            let ends = ends.clone();
            Large::new("railway's unsmoothed line", BAND, move |_, k| rule.points([ends.at(k), ends.at(k + 1)]))
        };
        let pieces = {
            let ends = ends.clone();
            Large::new("railway's line", BAND, move |_, k| {
                let n = 1i64 << LEVELS;
                let around: Vec<&Vec<[Fx; 3]>> = (k - 1..=k + 1).map(|k| raw.at(0, k)).collect();
                // Point i of this piece, past its ends those of its neighbours'.
                let unsmoothed = |i: i64| around[(i.div_euclid(n) + 1) as usize][i.rem_euclid(n) as usize];
                // MIN_RUN points past each end too, for the runs that cross it.
                let window = 2 * SMOOTH_HALF + 1;
                let all: Vec<[Fx; 3]> = (-MIN_RUN..=n + MIN_RUN)
                    .map(|i| {
                        let sum = (i - SMOOTH_HALF..=i + SMOOTH_HALF).map(|j| unsmoothed(j)[0]).fold(Fx::ZERO, |a, b| a + b);
                        let [_, y, z] = unsmoothed(i);
                        [sum / window, y, z]
                    })
                    .collect();
                let all: Vec<[Fx; 3]> = all.into_iter().map(|p| turned(&ends.terrain, p)).collect();
                let ground: Vec<Fx> = all.iter().map(|p| ends.terrain.ground(p[0].round_units().0, p[2].floor_units().0)).collect();
                let tops: Vec<Fx> = all.iter().map(|p| p[1]).collect();
                let inside = MIN_RUN as usize..=(n + MIN_RUN) as usize;
                let structure = structures(&ground, &tops)[inside.clone()].to_vec();
                let points = all[inside].to_vec();
                // Towns are further apart than twice the rows, and a row's nearest within them: each is some row's nearest.
                let (from, to) = (ends.z(k) - STATION - EASE - REACH, ends.z(k + 1) + STATION + EASE + REACH);
                let mut towns: Vec<Town> =
                    (from..to + TOWN_ROWS).step_by(TOWN_ROWS as usize).filter_map(|z| ends.terrain.towns.near(z)).collect();
                towns.dedup_by_key(|t| t.z);
                towns.retain(|t| (from..to).contains(&t.z));
                let xz = points.iter().map(|p| [p[0], p[2]]).collect();
                Piece { points, structure, towns, xz }
            })
        };
        let towns = ends.terrain.towns.clone();
        let line = Arc::new(Line { ends, pieces, towns, stations: scope.seed("stations"), surfaces: scope.seed("surfaces") });
        let at = {
            let line = line.clone();
            Plan::new("railway", SCALE, move |x, z| line.at(x, z))
        };
        Railway { line, at }
    }

    /** What column (x, z) is to the line, kept; columns surely out of its reach turned away before any plan is read. */
    fn at(&self, x: i64, z: i64) -> Option<At> {
        if !self.line.may_reach(x, z, REACH) {
            return None;
        }
        self.at.at(x, z)
    }

    /** Whether the ground it shapes reaches within `margin` of column (x, z): what keeps off it asks. */
    pub fn reaches(&self, x: i64, z: i64, margin: i64) -> bool {
        self.line.near(x, z, margin).is_some()
    }

    /** The first station from the start, ahead and behind in turn: the track by it. */
    pub fn first_station(&self) -> [i64; 3] {
        let line = &self.line;
        let rows = (0..1000).flat_map(|k| [k, -k - 1]).map(|k| m(200.0) + k * m(400.0));
        let s = rows.filter_map(|z| line.towns.near(z)).find_map(|t| line.station(&t)).expect("a station within 400 km");
        [s.at[0], s.top, s.at[1]]
    }

    /**
     * The first place from the start, ahead and behind in turn, where
     * `structure` carries the track: the track there. Within 256 km: much
     * further, the plans the ground is read by hold too few pieces.
     */
    pub fn first(&self, structure: Structure) -> [i64; 3] {
        let z = (0..m(256_000.0) / FINEST).flat_map(|k| [k, -k - 1]).map(|k| k * FINEST).find(|&z| self.line.track(z).2 == structure);
        let (at, top, _) = self.line.track(z.expect("one within 256 km"));
        [at[0], top, at[1]]
    }
}

impl Domain for Railway {
    /** Ahead over its formation and a little either side, but over a tunnel. */
    fn bid(&self, x: i64, z: i64) -> Fx {
        let most = fx(4.0);
        let Some(a) = self.at(x, z).filter(|a| a.structure != Structure::Tunnel) else { return -most };
        ((Units(a.half + m(2.0)).to_fx() - a.distance) * most / Units::m(2.0)).clamp(-most, most)
    }

    /**
     * Cut or filled to the track: level with it across the formation,
     * sloping from its edges, and round a bridge's ends under it as far as
     * its sides reach, which meet the ground there, not end in a wall.
     */
    fn shape(&self, x: i64, z: i64, ground: Fx) -> Option<(Fx, Fx)> {
        let (at, beyond) = self.at(x, z).and_then(|a| Some((a, a.beyond?)))?;
        let top = Units(at.top).to_fx();
        let out = (at.distance - Units(at.half).to_fx()).max(Fx::ZERO).hypot(beyond);
        let slope = out * fx(SLOPE);
        // Steeper over the last third of the sides, to meet the ground where they end.
        let sides = Units(SIDES).to_fx();
        Some((ground.clamp(top - slope, top + slope), Fx::ONE - smoothstep(sides * 2 / 3, sides, out)))
    }

    /** Its tunnels' bores, its track, bridges and platforms where it owns the land, its stations. */
    fn layers(self: Arc<Self>, ground: &Arc<Sloped>, hold: Holding) -> Vec<Layer> {
        layers(self, hold, ground.clone())
    }
}

fn layers(railway: Arc<Railway>, hold: Holding, ground: Arc<Sloped>) -> Vec<Layer> {
    // The track's top by row, rising under a unit a column (GRADE): a bore is over it, within BORE across.
    let top = {
        let line = railway.line.clone();
        Arc::new(Sloped::new(Arc::new(Plan::new("railway's top", SCALE, move |_, z| line.track(z).1)), 1))
    };
    let bores = {
        let (railway, top) = (railway.clone(), top.clone());
        // Last: nothing standing over a tunnel (trees and plants at a portal, mist) reaches into its bore.
        Field::columns("railway's tunnels", BORES, Op::Carve, move |x, z| {
            let Some(at) = railway.at(x, z).filter(|a| a.structure == Structure::Tunnel && a.distance < Units(BORE).to_fx()) else {
                return Vec::new();
            };
            vec![Span::solid(at.top + 1, at.top + 1 + arch(at.distance.floor_units().0), 0)]
        })
        .about(top, m(1.0), arch(0) + m(1.0))
    };
    let seed = railway.line.surfaces;
    // The track, and what carries it: in a tunnel, drawn after its bore.
    let track = |name, order, tunnel: bool| {
        let (railway, top) = (railway.clone(), top.clone());
        let (ground, hold) = (ground.clone(), hold.clone());
        Field::columns(name, order, Op::Paint, move |x, z| {
            let Some(at) = railway.at(x, z).filter(|a| (a.structure == Structure::Tunnel) == tunnel) else { return Vec::new() };
            let spacing = plan::spacing();
            let (top, e, floor) = (at.top, at.distance, bed(at.top));
            let within = |d: i64| e < Units(d).to_fx();
            let mut spans = Vec::new();
            match at.structure {
                // A deck with a parapet each side, on piers down to the ground.
                Structure::Bridge => {
                    if within(PIERS) && at.along.rem_euclid(PIER) < PIER_THICK {
                        spans.push(Span::solid(ground.at(x, z) - 2 * spacing, top - m(1.0), T::brick));
                    }
                    if within(DECK - 2) {
                        spans.push(Span::solid(top - m(1.0), floor, T::brick));
                    } else if within(DECK) {
                        // Its top where it is at every level: at least two sites, grown down, not up.
                        let parapet = top + 1 + m(0.75);
                        spans.push(Span::solid((top - m(1.0)).min(parapet - 2 * spacing), parapet, T::brick));
                    }
                }
                // A floor as deep as the ballast, and a lining round the bore: walls on it, then its vault.
                // The ground is far above: without a floor, the bore beside the ballast was open below.
                Structure::Tunnel if within(BORE + LINING) => {
                    let e = e.floor_units().0;
                    if e >= BORE {
                        spans.push(Span::solid(floor, top + 1 + arch(BORE - 1) + LINING, T::brick));
                    } else {
                        if !within(BALLAST) {
                            spans.push(Span::solid(floor, top + 1, F::path));
                        }
                        spans.push(Span::solid(top + 1 + arch(e), top + 1 + arch(e) + LINING, T::brick));
                    }
                }
                // The formation beside the ballast, or a platform at a station; where the railway owns it.
                Structure::Earth if !within(BALLAST) && within(at.half) && hold.owns(x, z) => {
                    if at.platform && !within(PLATFORM) && within(BUILDING_FROM) {
                        spans.extend(Column::new(top + PLATFORM_HIGH).surface(T::slab, PLATFORM_HIGH + m(0.5)).spans());
                    } else if within(FORMATION) || at.platform {
                        spans.extend(Column::new(top).surface(F::path, m(0.5)).spans());
                    }
                }
                _ => {}
            }
            if !within(BALLAST) {
                return spans;
            }
            // Ballast, sleepers and rails on it. Further, in the sleepers' strip, them as a medium over it as tall, as
            // Column::medium: stopping as much of a ray down as they cover, more of one coming in low, as they hide the
            // ballast; shorter than a spacing, its top as often of each as they cover (docs/todo.md).
            let (strip, coin) = (spacing > 1 && within(SLEEPERS), Fx::fraction(hash(seed, x, z, 0)));
            let thin = RAILS_HIGH < spacing;
            let surface: Kind = match coin {
                _ if !(strip && thin) => K::ballast.into(),
                coin if coin < fx(IRON) => T::iron.into(),
                coin if coin < fx(COVER) => T::timber.into(),
                _ => K::ballast.into(),
            };
            spans.push(Span::solid(floor, top + 1, surface));
            if spacing == 1 && (e - RAIL).abs() < Fx::HALF {
                spans.push(Span::solid(top + 1, top + 1 + RAILS_HIGH, T::iron));
            } else if spacing == 1 && within(SLEEPERS) && at.along.rem_euclid(SLEEPER) < SLEEPER_THICK {
                spans.push(Span::solid(top + 1, top + 1 + SLEEPER_THICK, T::timber));
            } else if strip && !thin {
                let kind = if coin < fx(IRON / COVER) { T::iron } else { T::timber };
                spans.push(Span::medium(top + 1, RAILS_HIGH, kind, -js::log(1.0 - COVER) / RAILS_HIGH as f64));
            }
            spans
        })
        // A tunnel's lining is the highest over the track; a bridge's piers stand on the ground, anywhere under it.
        .about(top, mountains::HEIGHTS.1 - mountains::HEIGHTS.0, arch(0) + LINING + 2)
    };
    let stations = {
        let line = railway.line.clone();
        let anchor = line.clone();
        Population::new("stations", 1.6, CELL)
            .reach(STATION_REACH)
            .anchor_along(1, move |cells| anchor.station_near(cells.centre[0], cells.centre[1]).map(|s| s.centre().map(|v| v as f64)))
            .make(move |_, c| line.station_near(c.centre[0], c.centre[2]).map(|s| building(&s)))
    };
    vec![
        Layer::Field(bores),
        Layer::Field(track("railway", 0.95, false)),
        Layer::Field(track("railway in its tunnels", BORES + 0.05, true)),
        Layer::Population(stations),
    ]
}

/** The bottom of the ballast under a track `top` high, and of what it lies on: half a metre down, two sites at least. */
fn bed(top: i64) -> i64 {
    top + 1 - m(0.5).max(2 * plan::spacing())
}

/** How high a tunnel's bore is `e` across from its middle: walls, then a round vault. */
fn arch(e: i64) -> i64 {
    WALLS + ((BORE * BORE - e * e) as u64).isqrt() as i64
}

/** A station's building: a house of a style its seed picks, its front to the track. */
fn building(s: &Station) -> Node {
    let mut shapes = Vec::new();
    House::new(BUILDING[0], BUILDING[1], 2, Style::of(s.seed)).build(&mut Part::new(s.frame(), &mut shapes), &mut Rng::new(s.seed));
    thing(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /** The railway of `seed`, and the terrain it is on. */
    fn railway(seed: u64) -> (Railway, Arc<Terrain>) {
        let world = Scope::new(seed);
        let terrain = Arc::new(Terrain::new(&world));
        (Railway::new(&world.of("railway"), terrain.clone()), terrain)
    }

    /**
     * Along its first 32 km at each seed: no segment climbs steeper than
     * the grade, the line keeps to its band across from the valley's
     * course (so out of towns), and it is bridged or tunnelled along
     * under 5 % of it, as it bends and climbs round what would need them.
     */
    #[test]
    fn keeps_its_grade_and_band_and_off_structures() {
        for seed in [42, 1, 2, 3] {
            let (railway, terrain) = railway(seed);
            let line = railway.line;
            let (mut costly, mut all) = (0, 0);
            for k in 0..8 {
                let piece = line.piece(k);
                for (i, w) in piece.points.windows(2).enumerate() {
                    let rise = (w[1][1] - w[0][1]).abs();
                    assert!(rise <= fx(GRADE) * Units(FINEST), "seed {seed}: piece {k} rises {rise:?} over a segment");
                    let across = (w[0][0] - terrain.course.x(w[0][2].floor_units().0)).round_units().0.abs();
                    assert!((INNER..=OUTER).contains(&across), "seed {seed}: piece {k} {across} across");
                    costly += usize::from(piece.structure[i] != Structure::Earth);
                    all += 1;
                }
            }
            assert!(costly * 20 < all, "seed {seed}: bridged or tunnelled at {costly} points of {all}");
        }
    }

    /**
     * Seen from above the line bends gently (no point turns by TURN from
     * the course, across the pieces' ends too, and the course turns less),
     * and no bridge or tunnel is shorter than MIN_RUN points.
     */
    #[test]
    fn is_smooth_and_has_no_stubs() {
        for seed in [42, 1, 2, 3] {
            let (railway, terrain) = railway(seed);
            let line = railway.line;
            let mut points: Vec<[Fx; 3]> = Vec::new();
            let mut structure: Vec<Structure> = Vec::new();
            for k in 0..8 {
                let piece = line.piece(k);
                points.extend(&piece.points[usize::from(k > 0)..]);
                structure.extend(&piece.structure[usize::from(k > 0)..]);
            }
            let across = |p: [Fx; 3]| p[0] - terrain.course.x(p[2].floor_units().0);
            let direction = |w: &[[Fx; 3]]| (across(w[1]) - across(w[0])).to_f64().atan2((w[1][2] - w[0][2]).to_f64());
            let turn = points.windows(3).map(|w| (direction(&w[1..]) - direction(w)).abs().to_degrees()).fold(0.0, f64::max);
            assert!(turn < TURN.to_degrees(), "seed {seed}: turns {turn}° at a point");
            // The first and last runs may go on past the pieces looked at.
            let runs: Vec<&[Structure]> = structure.chunk_by(|a, b| a == b).collect();
            for run in runs.iter().skip(1).take(runs.len().saturating_sub(2)) {
                assert!(run[0] == Structure::Earth || run.len() >= MIN_RUN as usize, "seed {seed}: a run of {}", run.len());
            }
        }
    }

    /**
     * Round the first few ends of bridges along the line, at each seed,
     * the railway steepens the ground by under a metre a unit: no fill
     * ends in a wall.
     */
    #[test]
    fn ends_its_fills_in_no_wall() {
        for seed in [42, 1, 2, 3] {
            let (railway, terrain) = railway(seed);
            let added =
                |x: i64, z: i64| railway.shape(x, z, terrain.ground(x, z)).map_or(Fx::ZERO, |(to, w)| (to - terrain.ground(x, z)) * w);
            let mut ends = 0;
            for k in 0.. {
                if ends >= 6 {
                    break;
                }
                let piece = railway.line.piece(k);
                for (i, w) in piece.structure.windows(2).enumerate() {
                    if w[0] == w[1] || w.contains(&Structure::Tunnel) {
                        continue;
                    }
                    ends += 1;
                    let [x0, _, z0] = piece.points[i].map(|v| v.round_units().0);
                    let r = FORMATION + SIDES + FINEST;
                    for (x, z) in (x0 - r..x0 + r).flat_map(|x| (z0 - r..z0 + r).map(move |z| (x, z))) {
                        let here = added(x, z);
                        let step = (added(x + 1, z) - here).abs().max((added(x, z + 1) - here).abs());
                        assert!(step < Fx::m(1.0), "seed {seed}: a step of {step:?} at ({x}, {z})");
                    }
                }
            }
        }
    }
}
