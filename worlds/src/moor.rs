//! The moor: rolling heath of heather and grass, a stream and pools in
//! its hollows, mist pooled in them, scattered boulders, and steadings: a
//! stone hut with a thatched roof, a ring of standing stones, and a path
//! of cairns between them. Written from the engine's public API alone
//! (docs/generators.md).
//!
//! A domain too, like the forest: its layers lie on ground decided outside
//! it, where it owns the land (`Ground`), and what it places, steadings and
//! pools, only where it surely will (`Moorland`). The valley (valley.rs)
//! has it on its uplands and along stretches of its floor (`Uplands`);
//! `moor` is the moor alone, all of its land, with its own stream.

use std::rc::Rc;

use worldgen::World;
use worldgen::column::Column;
use worldgen::frame::{Frame, Part, facing, walls};
use worldgen::generator::{ANY_SIZE, Child, Far, Field, Generator, Kind, Layer, Node, Op, Population, Shape, Span};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, ease_towards, fx, hash64, smoothstep, stream};
use worldgen::plan::{Claims, Land, Plan};
use worldgen::units::{Units, m};

use crate::forest::K as F;
use crate::landmarks::{self, Landmarks};
use crate::mountains::K as A;
use crate::path::Path;
use crate::railway::Railway;
use crate::town::K as T;
use crate::valley::{self, Terrain};

// After the valley's kinds, the moor's own; the rest are the forest's stone, soil and path,
// the town's thatch, timber and water, the mountains' grass.
kinds! {
    base: landmarks::BASE + landmarks::KINDS.len() as u8;
    heather: 5, ["#6b3a5e", "#7a4569", "#84506f"];
    bog: 1, ["#5a7a3a", "#4f6e33"];
    lichen: 3, ["#9a9a7a", "#a6a682"];
    drystone: 3, ["#6f6a62", "#7a756c"];
    // What stands on the heath and the grass, in their colours: walked through, the ground not.
    heatherSprigs: 5, ["#6b3a5e", "#7a4569", "#84506f"], walk_through;
    grassBlades: 1, ["#8fa052", "#9aab5c"], walk_through;
    mist: 8, ["#c9d0d4", "#c4cbd0", "#ced4d8"], walk_through;
}

/** The heights the moor alone has anything at (lattice units). */
const HEIGHTS: (i64, i64) = (m(-24.0), m(32.0));

/** Water stands in the moor alone's hollows up to this height. */
const WATER: Units = Units::m(-4.5);
/** Bog this high above water too. */
const BOG: Units = Units::m(0.25);
/** Plan scales: the stream's bends, its banks, the ground's bumps. */
const STREAM_SCALE: i64 = m(32.0);
const BANK_SCALE: i64 = m(1.0);
const GROUND_SCALE: i64 = 8;
/**
 * Steadings' rings stand at least APART, claimed (Claims) from spots one
 * in each square CLAIM wide on average; a steading reaches NEAR from its
 * ring at most (its hut, the ground it levels, its path), so a column is
 * near one at most. The first's ring is at FIRST.
 */
const CLAIM: i64 = m(128.0);
const APART: i64 = m(112.0);
pub const NEAR: i64 = m(56.0);
const FIRST: [i64; 2] = [m(8.0), m(10.0)];
/** Pools' centres stand at least POOLS_APART, claimed from spots one in each square POOL_CLAIM wide on average. */
const POOL_CLAIM: i64 = m(64.0);
const POOLS_APART: i64 = m(32.0);
/** A steading's cell, and how far its things reach past it. */
const STEADING_CELL: i64 = m(32.0);
/** Hut from ring, at most; ring radius, at most; hut's half diagonal, at most. */
const HUT_FROM_RING: i64 = m(30.0);
const RING_RADIUS: i64 = m(10.0);
const HUT_HALF: i64 = m(5.0);
/** How far a steading reaches past its cell across: to the hut's far corner, or the ring's far stone. */
const REACH_ACROSS: i64 = if HUT_FROM_RING + HUT_HALF > RING_RADIUS { HUT_FROM_RING + HUT_HALF + m(2.0) } else { RING_RADIUS + m(2.0) };
/** How far the ground eases from a hut's level back to the heath's. */
const TERRACE: Units = Units::m(12.0);
// The levelled ground's far corner (a square round the hut) is the furthest a steading reaches from its ring.
const _: () = assert!(HUT_FROM_RING + (HUT_HALF + TERRACE.0) * 3 / 2 <= NEAR && 2 * NEAR <= APART);
/** A pool's radius at most: pools stand at least two apart, so a column is in one at most. */
const POOL_RADIUS: i64 = m(14.0);
const _: () = assert!(2 * POOL_RADIUS <= POOLS_APART);
/** How far the moor's bid leads the rest where it has room for a steading or a pool: the land's width and some. */
const SURE: Fx = fx(0.2);
/**
 * Mist fills a hollow up to MIST_UNDER under the mean of the ground on a
 * square MIST_RING round it, and lies up to MIST_OVER higher in patches;
 * its top made every MIST_STEP. So little of it lies thin on slopes,
 * where it would be thickened far away ("Thickened media spread",
 * generators.md). As thick as some 15 m of it hide what is behind
 * (Koschmieder: 3.9 over the distance seen).
 */
const MIST_RING: i64 = m(24.0);
const MIST_UNDER: Units = Units::m(1.0);
const MIST_OVER: Units = Units::m(3.0);
const MIST_STEP: i64 = m(8.0);
// The ring's points are samples' too: the ground is made once for each.
const _: () = assert!(MIST_RING % MIST_STEP == 0);
const MIST: f64 = 3.9 / m(15.0) as f64;
const WALL: i64 = 3;

/** A quantity of column (x, z) from position alone, with its fraction: the ground's height, a bid's lead. */
type ByColumn = Rc<dyn Fn(i64, i64) -> Fx>;

/** A steading: a ring of stones, a hut facing it (its front wall towards the ring), a path between. */
#[derive(Clone, Copy, Default)]
struct Steading {
    ring: [i64; 2],
    ring_ground: Units,
    radius: i64,
    stones: i64,
    /** The hut's footprint centre (x, z), floor level, footprint and quarter turns. */
    centre: [i64; 2],
    level: Units,
    length: i64,
    depth: i64,
    turns: i64,
    seed: u64,
}

impl Steading {
    fn frame(&self) -> Frame {
        Frame::centred([self.centre[0], self.level.0 + 1, self.centre[1]], [self.length, 0, self.depth], self.turns)
    }

    /** Where the path starts and ends: outside the hut's door, and at the ring's edge. */
    fn path(&self) -> ([f64; 2], [f64; 2]) {
        let [x, _, z] = self.frame().site([self.length / 2, 0, -m(1.0)]);
        let (dx, dz) = ((self.ring[0] - x) as f64, (self.ring[1] - z) as f64);
        let d = js::hypot(&[dx, dz]);
        let end = (d - self.radius as f64 + m(1.5) as f64).max(0.0) / d;
        ([x as f64, z as f64], [x as f64 + dx * end, z as f64 + dz * end])
    }

    /** The ground's height at (x, z), `heath` there, levelled under its hut and easing back over TERRACE. */
    fn terraced(&self, x: i64, z: i64, heath: Fx) -> Fx {
        let c = self.centre;
        let d = Units((x - c[0]).abs().max((z - c[1]).abs()) - HUT_HALF);
        ease_towards(heath, self.level.to_fx(), TERRACE, d)
    }
}

/** A pool in a hollow, dug `dig` deeper at its centre: water up to `level` where the ground is under it, within `radius` of its centre. */
#[derive(Clone, Copy, Default)]
struct Pool {
    centre: [i64; 2],
    radius: i64,
    level: Units,
    dig: Units,
}

impl Pool {
    /** How far column (x, z) is inside it, a fraction: none at its rim and beyond, all at its centre. */
    fn inside(&self, x: i64, z: i64) -> Fx {
        let [cx, cz] = self.centre;
        let d2 = (x - cx).pow(2) + (z - cz).pow(2);
        if d2 >= self.radius.pow(2) { Fx::ZERO } else { Fx::ONE - Units(d2).to_fx() / Units(self.radius.pow(2)) }
    }

    /** `ground` at (x, z), with its fraction, hollowed: deepest at the centre, easing to nothing at the rim. */
    fn hollowed(&self, x: i64, z: i64, ground: Fx) -> Fx {
        let inside = self.inside(x, z);
        ground - inside * inside * self.dig
    }
}

/** Distance from (x, z) to the segment from a to b. */
fn to_segment(x: f64, z: f64, a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 == 0.0 { 0.0 } else { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) };
    js::hypot(&[x - a[0] - t * dx, z - a[1] - t * dz])
}

/** What a column is by the stream: in its bed, on its boggy banks a metre either side, or neither. */
#[derive(Clone, Copy, Default, PartialEq)]
enum Streamside {
    #[default]
    Off,
    Bank,
    Bed,
}

/** The moor alone's stream: its centre line by z, and where its bed and banks are, dithered far away, so it shows as often as it is. */
struct Stream {
    line: Rc<Plan<Fx>>,
    side: Plan<Streamside>,
}

/** Where water stands: in every hollow up to a level, or in pools in hollows where the moor has room for them. */
enum Water {
    Level(Units),
    Pools(Claims<Pool>),
}

/** How far column (x, z) is from the centre line of `stream`; and the stream's half width there. */
fn stream_distance(stream: &Plan<Fx>, x: i64, z: i64) -> (Units, Units) {
    ((Units(x).to_fx() - stream.at(0, z)).abs().floor_units(), Units::m(1.0) + Units::m(0.6) * z.rem_euclid(97) / 97)
}

/**
 * The moor's decisions from position alone, coarser than any column:
 * its patches of heather, its stream, if it has one, its steadings and
 * its water. A host whose ground they stand on levels it under the huts
 * and hollows it under the pools (`shaped`).
 */
pub struct Moorland {
    seed: u64,
    patches: Fbm<{ m(30.0) }, 2>,
    /** Where mist lies deeper. */
    mist: ValueNoise<{ m(48.0) }>,
    stream: Option<Stream>,
    steadings: Rc<Claims<Steading>>,
    water: Water,
}

impl Moorland {
    /**
     * The moorland of `seed` on ground `height` high (before it levels
     * it), with steadings and pools where `lead`, by how much its bid
     * leads every other domain's at least, is SURE: their columns are its.
     * Steadings claimed from spots by `chance`; a stream along `line`, if
     * any; water in every hollow up to `level`, or without one, in pools.
     */
    fn new(seed: u64, height: ByColumn, lead: ByColumn, chance: Fx, line: Option<Rc<dyn Fn(i64) -> Fx>>, level: Option<Units>) -> Moorland {
        let brook = line.clone().map(|line| {
            let line = Rc::new(Plan::new("stream", STREAM_SCALE, move |_, z| line(z)));
            let side = {
                let line = line.clone();
                Plan::dithered("streamside", BANK_SCALE, move |x, z| {
                    let (d, half) = stream_distance(&line, x, z);
                    if d < half {
                        Streamside::Bed
                    } else if d < half + Units::m(1.0) {
                        Streamside::Bank
                    } else {
                        Streamside::Off
                    }
                })
            };
            Stream { line, side }
        });
        let steadings = {
            let (height, lead) = (height.clone(), lead.clone());
            let s = stream(seed, 5);
            Rc::new(Claims::new("steadings", s, CLAIM, APART, Some(FIRST), move |ring, random| {
                // The stream's plan is finer than the steadings': its line is read here.
                let off_stream = |c: [i64; 2]| line.as_ref().is_none_or(|l| (Units(c[0]).to_fx() - l(c[1])).abs() > Units::m(14.0).to_fx());
                let steading = if ring == FIRST {
                    steading_at(ring, true, &mut Rng::new(hash64(s, 0, 0, 0)), &*height)
                } else if random.chance(chance) {
                    steading_at(ring, false, random, &*height)
                } else {
                    return None;
                };
                let low = level.is_some_and(|w| steading.level <= w + Units::m(1.0));
                // Room all round the hut and the ring.
                let around = |c: [i64; 2]| (-1..=1).all(|a| (-1..=1).all(|b| lead(c[0] + a * m(20.0), c[1] + b * m(20.0)) >= SURE));
                (!low && off_stream(steading.centre) && off_stream(steading.ring) && around(steading.centre) && around(steading.ring))
                    .then_some((ring, steading))
            }))
        };
        let water = match level {
            Some(level) => Water::Level(level),
            None => {
                let steadings = steadings.clone();
                Water::Pools(Claims::new("pools", stream(seed, 7), POOL_CLAIM, POOLS_APART, None, move |spot, random| {
                    if !random.chance(fx(0.35)) {
                        return None;
                    }
                    let ground = |x, z| terraced(&steadings, x, z, height(x, z));
                    let pool = pool_at(spot, &ground, random)?;
                    let [x, z] = pool.centre;
                    let r = pool.radius + m(4.0);
                    let room = [(0, 0), (r, 0), (-r, 0), (0, r), (0, -r)].iter().all(|(a, b)| lead(x + a, z + b) >= SURE);
                    // Off all the steadings reach: theirs is levelled ground, and their huts and paths dry.
                    let off = steadings.near(x, z, r + NEAR).is_none();
                    (room && off).then_some((pool.centre, pool))
                }))
            }
        };
        Moorland {
            seed: stream(seed, 8),
            patches: Fbm::new(stream(seed, 3)),
            mist: ValueNoise::new(stream(seed, 9)),
            stream: brook,
            steadings,
            water,
        }
    }

    /** `ground` at (x, z), with its fraction, levelled under the hut of a steading there, hollowed under a pool. */
    pub fn shaped(&self, x: i64, z: i64, ground: Fx) -> Fx {
        let ground = terraced(&self.steadings, x, z, ground);
        self.pool(x, z).map_or(ground, |pool| pool.hollowed(x, z, ground))
    }

    /** The pool column (x, z) may be in, if any. */
    fn pool(&self, x: i64, z: i64) -> Option<Pool> {
        let Water::Pools(pools) = &self.water else { return None };
        pools.near(x, z, POOL_RADIUS).map(|(_, pool)| pool)
    }

    /** Where the steading reaching column (x, z) is, if any: between its hut and its ring, at the hut's floor. */
    pub fn steading_near(&self, x: i64, z: i64) -> Option<[i64; 3]> {
        let s = self.steading(x, z)?;
        Some([(s.centre[0] + s.ring[0]).div_euclid(2), s.level.0, (s.centre[1] + s.ring[1]).div_euclid(2)])
    }

    /** The steading reaching column (x, z), if any: its ring within NEAR. */
    fn steading(&self, x: i64, z: i64) -> Option<Steading> {
        self.steadings.near(x, z, NEAR).map(|(_, s)| s)
    }

    fn streamside(&self, x: i64, z: i64) -> Streamside {
        self.stream.as_ref().map_or(Streamside::Off, |s| s.side.at(x, z))
    }

    /** Whether column (x, z) is on the path of the steading reaching it. */
    fn on_path(&self, x: i64, z: i64) -> bool {
        self.steading(x, z).is_some_and(|s| {
            let (a, b) = s.path();
            to_segment(x as f64, z as f64, a, b) < m(0.7) as f64
        })
    }

    /** The level water stands to in column (x, z), where it is under it. */
    fn water(&self, x: i64, z: i64) -> Option<Units> {
        match &self.water {
            Water::Level(level) => Some(*level),
            Water::Pools(_) => self.pool(x, z).and_then(|pool| (pool.inside(x, z) > Fx::ZERO).then_some(pool.level)),
        }
    }
}

/** `ground` at (x, z), levelled under the hut of the steading reaching it in `steadings`. */
fn terraced(steadings: &Claims<Steading>, x: i64, z: i64, ground: Fx) -> Fx {
    steadings.near(x, z, NEAR).map_or(ground, |(_, s)| s.terraced(x, z, ground))
}

/** The steading whose ring is at `ring`, the first if `first`: its hut 22 to 30 m off it, on ground `height` high. */
fn steading_at(ring: [i64; 2], first: bool, random: &mut Rng, height: &dyn Fn(i64, i64) -> Fx) -> Steading {
    // The first steading's hut east of its ring, away from the stream.
    let turn = if first { 0.6 * random.next() - 0.3 } else { random.next() };
    let angle = turn * 2.0 * std::f64::consts::PI;
    let from = m(22.0) as f64 + random.next() * m(8.0) as f64;
    let centre = [ring[0] + (from * js::cos(angle)) as i64, ring[1] + (from * js::sin(angle)) as i64];
    let (length, depth) = (m(5.0) + random.below(m(1.5) as u32) as i64, m(4.0) + random.below(m(1.0) as u32) as i64);
    // Its front faces the ring.
    let turns = facing(ring[0] - centre[0], ring[1] - centre[1]);
    // A hut's level is the mean of the heath under it, as coarse as the hut.
    let mean = |c: [i64; 2]| {
        let around = (-1..=1).flat_map(|a| (-1..=1).map(move |b| (a, b)));
        Fx::mean(around.map(|(a, b)| height(c[0] + a * m(4.0), c[1] + b * m(4.0)))).round_units()
    };
    let level = mean(centre);
    let ring_ground = mean(ring);
    let radius = m(7.0) + random.below(m(3.0) as u32) as i64;
    let stones = 9 + random.below(5) as i64;
    Steading { ring, ring_ground, radius, stones, centre, level, length, depth, turns, seed: random.next_u64() }
}

/**
 * The pool claimed from `spot`, if the ground lets it: in the lowest of a
 * few points within 16 m of it, as wide as the ground round it is level
 * enough, up to POOL_RADIUS; water up to half a metre under the lowest of
 * that ground, so it and its boggy shore stay inside it, the hollow dug
 * deeper where it is shallow.
 */
fn pool_at(spot: [i64; 2], ground: &dyn Fn(i64, i64) -> Fx, random: &mut Rng) -> Option<Pool> {
    let points = (-2..=2).flat_map(|a| (-2..=2).map(move |b| [spot[0] + a * m(8.0), spot[1] + b * m(8.0)]));
    let centre = points.min_by_key(|[x, z]| ground(*x, *z))?;
    let bottom = ground(centre[0], centre[1]);
    let smallest = m(4.0) + random.below(m(4.0) as u32) as i64;
    (0..4).map(|n| POOL_RADIUS - n * (POOL_RADIUS - smallest) / 3).find_map(|radius| {
        let rim: Vec<Fx> = (0..32)
            .map(|a| {
                let angle = a as f64 * std::f64::consts::PI / 16.0;
                ground(centre[0] + (radius as f64 * js::cos(angle)) as i64, centre[1] + (radius as f64 * js::sin(angle)) as i64)
            })
            .collect();
        let (low, high) = (*rim.iter().min()?, *rim.iter().max()?);
        let level = (low - Fx::m(0.5)).floor_units();
        let dig = (bottom - (level - Units::m(0.8)).to_fx()).max(Fx::ZERO).round_units();
        (high - low < Fx::m(1.5) && dig < Units::m(1.5)).then_some(Pool { centre, radius, level, dig })
    })
}

/**
 * The moor's bid for the valley's land (valley.rs), from position alone:
 * on the uplands under the treeline, and over stretches of the valley a
 * kilometre or so long and kilometres apart; patchy, so copses of the
 * forest stand in it and heath in the forest, its edges ragged. None
 * between `clear`'s two z, nor on the towns' farmland. Steadings and
 * pools keep off the towns, the landmarks, the path and the railway (`lead`).
 */
pub struct Uplands {
    terrain: Rc<Terrain>,
    landmarks: Rc<Landmarks>,
    railway: Rc<Railway>,
    clear: [i64; 2],
    stretches: ValueNoise<{ m(1600.0) }>,
    patches: Fbm<{ m(160.0) }, 2>,
}

impl Uplands {
    pub fn new(seed: u64, terrain: Rc<Terrain>, landmarks: Rc<Landmarks>, railway: Rc<Railway>, clear: [i64; 2]) -> Uplands {
        Uplands { terrain, landmarks, railway, clear, stretches: ValueNoise::new(stream(seed, 80)), patches: Fbm::new(stream(seed, 81)) }
    }

    /** Its bid for column (x, z), from position alone. */
    pub fn bid(&self, x: i64, z: i64) -> Fx {
        let h = self.terrain.ground(x, z).round_units().0;
        let (ranges, units) = (&self.terrain.ranges, |d: i64| Units(d).to_fx());
        // From 65 m under the treeline, rising over 30 m; giving way to the mountains' meadows round the treeline, none above.
        let uplands = Fx::ONE - ranges.below_treeline(x, z, h + m(35.0));
        let below = ranges.below_treeline(x, z, h - m(10.0));
        // Stretches across the whole valley: their noise four times as long across it.
        let patches = self.patches.at(x, z) - Fx::HALF;
        let stretch = smoothstep(fx(0.76), fx(0.84), self.stretches.at(x / 4, z) + patches / 2);
        let moorish = uplands.max(stretch) * below;
        let [from, to] = self.clear;
        let clear = Fx::ONE - smoothstep(Fx::ZERO, Fx::m(150.0), units((from - z).max(z - to)));
        // The towns' farmland reaches 90 to 150 m out of them (fields.rs).
        let farmland = Fx::ONE - smoothstep(Fx::m(90.0), Fx::m(150.0), self.town_distance(x, z));
        (moorish - Fx::HALF) * 4 / 5 + patches * 6 / 5 - (clear + farmland + Fx::ONE - below) * 2
    }

    /** How far column (x, z) is from the nearest town, up to 300 m. */
    fn town_distance(&self, x: i64, z: i64) -> Fx {
        self.terrain.towns.distance(x, z).min(Units::m(300.0)).to_fx()
    }

    /**
     * By how much its bid leads every other domain's at least: off the
     * towns and their fields, and some way off the landmarks' clearings,
     * the others bid nothing (the forest); less by the path and the
     * railway, which steadings and pools keep off.
     */
    fn lead(&self, x: i64, z: i64) -> Fx {
        let from_path = (Units(x).to_fx() - self.terrain.path_x(z)).abs();
        let towns = Fx::ONE - smoothstep(Fx::m(200.0), Fx::m(300.0), self.town_distance(x, z));
        // The landmarks' bid rises by one every 2 m into their clearings from -4, 8 m out.
        let landmark = if self.landmarks.bid(x, z) > fx(-3.75) { Fx::ONE } else { Fx::ZERO };
        let railway = if self.railway.reaches(x, z, m(8.0)) { Fx::ONE } else { Fx::ZERO };
        self.bid(x, z) - (Fx::ONE - smoothstep(Fx::m(25.0), Fx::m(40.0), from_path) + towns + landmark + railway) * 2
    }

    /** The moorland on them, of `seed`: steadings, fewer than on the moor alone, and pools where it leads; no stream. */
    pub fn moorland(self: &Rc<Self>, seed: u64) -> Moorland {
        let (terrain, lead) = (self.terrain.clone(), self.clone());
        Moorland::new(seed, Rc::new(move |x, z| terrain.ground(x, z)), Rc::new(move |x, z| lead.lead(x, z)), fx(0.225), None, None)
    }
}

/**
 * What the moor lies on, decided outside it when it shares its land:
 * the ground's height, the land and its index in it if it shares one (it
 * draws what it owns, and its boulders are as many as its share), a path
 * through it that others draw, and where its peat ends below if nothing
 * is under it.
 */
pub struct Ground {
    pub ground: Rc<Plan<i64>>,
    pub land: Option<(Rc<Land>, usize)>,
    pub path: Option<Rc<Path>>,
    pub bottom: Option<i64>,
}

/** The moor's layers' state: its moorland, and what it lies on. */
struct Moor {
    moorland: Rc<Moorland>,
    ground: Ground,
}

impl Moor {
    fn height(&self, x: i64, z: i64) -> i64 {
        self.ground.ground.at(x, z)
    }

    fn owns(&self, x: i64, z: i64) -> bool {
        self.ground.land.as_ref().is_none_or(|(land, me)| land.owns(*me, x, z))
    }

    fn share(&self, x: i64, z: i64) -> Fx {
        self.ground.land.as_ref().map_or(Fx::ONE, |(land, me)| land.share(*me, x, z))
    }

    /** Whether column (x, z) is within `distance` of the path through it, if any. */
    fn by_path(&self, x: i64, z: i64, distance: Fx) -> bool {
        self.ground.path.as_ref().is_some_and(|p| p.by(x, z, distance))
    }
}

/** The moor of `seed`, all of it; the camera starts behind the first steading's hut, looking past it to the ring. */
pub fn moor(seed: u64) -> World {
    let hills = Fbm::<{ m(240.0) }, 4>::new(stream(seed, 1));
    let bumps = ValueNoise::<{ m(10.0) }>::new(stream(seed, 2));
    let heath: ByColumn = Rc::new(move |x, z| (hills.at(x, z) - Fx::HALF) * Units::m(28.0) + (bumps.at(x, z) - Fx::HALF) * Units::m(0.8));
    // Winding along z west of the origin.
    let phase = (hash64(stream(seed, 4), 0, 0, 0) % 1000) as f64 / 100.0;
    let line: Rc<dyn Fn(i64) -> Fx> = Rc::new(move |z| {
        let zm = z as f64 / m(1.0) as f64;
        fx(m(-40.0) as f64 + m(1.0) as f64 * (12.0 * js::sin(zm / 37.0 + phase) + 4.0 * js::sin(zm / 11.0 + 2.0 * phase)))
    });
    let moorland = Rc::new(Moorland::new(seed, heath.clone(), Rc::new(|_, _| Fx::ONE), fx(0.65), Some(line), Some(WATER)));
    // The heath, levelled under each hut.
    let ground = {
        let moorland = moorland.clone();
        Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| moorland.shaped(x, z, heath(x, z)).round_units().0))
    };
    let spawn = {
        // Behind the first hut, or over the origin if there is none.
        let [x, _, z] = moorland.steading(0, 0).map_or([0; 3], |s| s.frame().site([s.length / 2, 0, s.depth + m(6.0)]));
        let y = Units(ground.at(x, z)).max(WATER).0 + m(2.0);
        [x, y + ((x + y + z) & 1), z]
    };
    let layers = layers(moorland, Ground { ground, land: None, path: None, bottom: Some(HEIGHTS.0) });
    // The valley's kinds, its own last.
    World { generator: Generator { kinds: valley::kinds(), seed: stream(seed, 6), layers }, spawn, heights: HEIGHTS, points: Vec::new() }
}

/** The moor's layers on `ground`, of `moorland`: drawn where it owns the land; its kinds are KINDS, after the valley's. */
pub fn layers(moorland: Rc<Moorland>, ground: Ground) -> Vec<Layer> {
    let f = Rc::new(Moor { moorland, ground });
    let mut layers = vec![Layer::Field(heath(f.clone()))];
    if f.moorland.stream.is_some() {
        layers.push(Layer::Field(stream_bed(f.clone())));
    }
    layers.extend([
        Layer::Field(water(f.clone())),
        Layer::Population(boulders(f.clone())),
        Layer::Population(steadings_population(f.clone())),
        Layer::Field(mist(f)),
    ]);
    layers
}

/**
 * Mist: cold air pooled in the hollows, its top level over each, and in
 * patches over the rest; as deep as the moor's share, so it thins out
 * where the moor gives way. Drawn after all else, into what is empty, so
 * it stands among the heather, round the huts and over the water, and
 * takes nothing's place.
 */
fn mist(f: Rc<Moor>) -> Field {
    let lid = {
        let ground = f.ground.ground.clone();
        Plan::sampled("mist", MIST_STEP, MIST_STEP, move |x, z| {
            let r = MIST_RING;
            let ring = [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (r, -r), (-r, r), (-r, -r)];
            Fx::mean(ring.map(|(dx, dz)| Units(ground.at(x + dx, z + dz)).to_fx()))
        })
    };
    Field::columns("mist", 5.0, Op::Fill, move |x, z, spacing| {
        let share = f.share(x, z);
        if share == Fx::ZERO {
            return Vec::new();
        }
        let h = f.height(x, z);
        let top = lid.at(x, z) - MIST_UNDER.to_fx() + smoothstep(fx(0.35), fx(0.75), f.moorland.mist.at(x, z)) * MIST_OVER;
        let depth = ((top - Units(h).to_fx()) * share).round_units().0;
        if depth <= 0 {
            return Vec::new();
        }
        vec![Span::medium(h + 1, depth, K::mist, MIST, spacing)]
    })
}

/** The heath: heather, grass or bog over peat, a gravel path to each ring; heather and grass standing on it as media. */
fn heath(f: Rc<Moor>) -> Field {
    Field::columns("heath", 0.7, Op::Paint, move |x, z, spacing| {
        if !f.owns(x, z) || f.by_path(x, z, Fx::m(1.6)) {
            return Vec::new();
        }
        let h = f.height(x, z);
        let wet = f.moorland.water(x, z).is_some_and(|level| Units(h) <= level + BOG) || f.moorland.streamside(x, z) != Streamside::Off;
        let surface: Kind = if wet {
            K::bog.into()
        } else if f.moorland.on_path(x, z) {
            F::path.into()
        } else if f.moorland.patches.at(x, z) > fx(0.47) {
            K::heather.into()
        } else {
            A::alpine.into()
        };
        let column = Column::new(h, spacing).surface(surface, m(0.5));
        // Peat down to the world's bottom and no further, where hosts stop generating: the heath is solid, not a shell.
        // On another's ground, half a metre of it.
        let column = match f.ground.bottom {
            Some(bottom) => column.fill_below_exactly(F::soil, bottom),
            None => column.surface(F::soil, m(0.5)),
        };
        // Thickened only while it is at most four units: taller, a medium stops rays skimming over the heath,
        // and stands over the patches beside it, whose edges it takes over seen from above.
        if spacing > 2 || wet {
            column
        } else if surface == K::heather.into() {
            column.medium(K::heatherSprigs, m(0.4), 0.5)
        } else if surface == A::alpine.into() {
            column.medium(K::grassBlades, 2, 0.25)
        } else {
            column
        }
        .spans()
    })
}

/** The bottom of the stream's bed under ground `h` high: a metre deep, at least two sites, as a line of sites along y has one every two. */
fn bed_bottom(h: i64, spacing: i64) -> i64 {
    h - m(1.0).max(2 * spacing)
}

/**
 * The stream's bed: the ground carved out down to where `water`, drawn
 * after, fills it from; on the faces they share, they keep the same sites.
 */
fn stream_bed(f: Rc<Moor>) -> Field {
    Field::columns("stream bed", 1.0, Op::Carve, move |x, z, spacing| {
        if f.moorland.streamside(x, z) != Streamside::Bed {
            return Vec::new();
        }
        let h = f.height(x, z);
        vec![Span::solid(bed_bottom(h, spacing), h + 1, 0)]
    })
}

/**
 * Water: in the stream's bed, a few units under its banks, and in the
 * hollows. Painted, so at coarse levels, where the ground's surface is
 * two sites thick, its top site is water, not lost under it.
 */
fn water(f: Rc<Moor>) -> Field {
    Field::columns("water", 2.0, Op::Paint, move |x, z, spacing| {
        let h = f.height(x, z);
        if f.moorland.streamside(x, z) == Streamside::Bed {
            return vec![Span::solid(bed_bottom(h, spacing), h - 1, T::water)];
        }
        match f.moorland.water(x, z) {
            Some(level) if f.owns(x, z) => Column::new(h, spacing).water_to(T::water, level.0).spans(),
            _ => Vec::new(),
        }
    })
}

/** Boulders half sunk in the heath, granite or lichened, off the water and the paths; as many as the moor's share. */
fn boulders(f: Rc<Moor>) -> Population {
    let ground = f.clone();
    Population::new("moor boulders", 3.0, m(6.0))
        .reach(m(2.5))
        .anchor_along(1, move |line| {
            let [i, k] = line.across;
            Some(line.pick(&mut Rng::new(hash64(ground.moorland.seed, i, 0, k)), |x, z| ground.height(x, z)))
        })
        .make(move |[x, y, z], c| {
            let mut random = c.random("boulder");
            let (xi, zi) = (x as i64, z as i64);
            let chance = fx(0.1) * f.share(xi, zi);
            let wet = f.moorland.water(xi, zi).is_some_and(|level| y <= level.0 as f64 + 2.0);
            let by_stream = f.moorland.stream.as_ref().is_some_and(|s| stream_distance(&s.line, xi, zi).0 < Units::m(4.0));
            if !random.chance(chance) || wet || by_stream || f.moorland.on_path(xi, zi) || f.by_path(xi, zi, Fx::m(3.0)) {
                return None;
            }
            let r = m(0.4) as f64 + random.next() * m(0.8) as f64;
            let radii = [r * (1.0 + random.next() * 0.6), r * 0.7, r * (1.0 + random.next() * 0.6)];
            let kind: Kind = if random.chance(fx(1.0 / 3.0)) { K::lichen.into() } else { F::stone.into() };
            Some(Node::of(vec![Shape::ellipsoid([x, y + r * 0.25, z], radii, kind)]))
        })
}

/** The steadings: each owned by the cell its ring's centre is in. */
fn steadings_population(f: Rc<Moor>) -> Population {
    let anchor = f.clone();
    Population::new("steadings", 4.0, STEADING_CELL)
        .reach([REACH_ACROSS, m(16.0), REACH_ACROSS])
        .anchor_along(1, move |line| {
            let s = anchor.moorland.steading(line.centre[0], line.centre[1])?;
            Some([s.ring[0] as f64, s.ring_ground.0 as f64, s.ring[1] as f64])
        })
        .make(move |_, c| f.moorland.steading(c.centre[0], c.centre[2]).map(|s| steading(&f, &s)))
}

/** A steading's node: the hut, the ring of stones and the cairns, each a child. */
fn steading(f: &Moor, s: &Steading) -> Node {
    let mut random = Rng::new(s.seed);
    let hut = hut(s, &mut random);
    let ring = ring(f, s, &mut random);
    let cairns = cairns(f, s, &mut random);
    Node::group(vec![Child::new("hut", ANY_SIZE, hut), Child::new("ring", ANY_SIZE, ring), Child::new("cairns", m(2.0), cairns)])
}

/** The hut, in its frame: u along its front, v up from its floor, w back from the front. Drystone walls, a doorway and a window cut through them, a thatched gable roof, a hearth. */
fn hut(s: &Steading, random: &mut Rng) -> Node {
    let (length, depth) = (s.length, s.depth);
    let top = m(2.0);
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    part.add([-1, -m(1.5), -1], [length + 1, 0, depth + 1], F::stone);
    part.add([0, 0, 0], [length, top, depth], K::drystone);
    part.cut([WALL, 0, WALL], [length - WALL, top, depth - WALL]);
    let [(front, _), _, (back, back_length), _] = walls(length, depth);
    part.within(front).cut([length / 2 - 4, 0, 0], [length / 2 + 4, m(1.7), WALL]);
    part.within(back).cut([back_length / 2 - 2, 7, 0], [back_length / 2 + 2, 11, WALL]);
    // A lintel over the door, a hearth on the floor.
    part.within(front).add([length / 2 - 6, m(1.7), -1], [length / 2 + 6, m(1.7) + 2, 0], T::timber);
    part.ball([(length / 2) as f64, 0.5, (depth * 2 / 3) as f64], 2.5, F::stone, Op::Fill);
    // A thatched roof, one solid, overhanging all round: a skin of thatch over a stone gable
    // would be thinner than coarse sites, and they would show the stone.
    let rise = (depth as f64 / 2.0 + 3.0) * (0.9 + random.next() * 0.3);
    part.gable([-2.0, (top - 2) as f64, -3.0], [(length + 2) as f64, top as f64 + rise, (depth + 3) as f64], 0, T::thatch, Op::Fill);
    // From afar, its outline. Measured (Far::measured), the roof's thatch is lost from above at levels 2 and 3
    // and what rays hit differs ~0.03 more at levels 2 and 3 in the contract.
    let mut far = Vec::new();
    let mut outline = Part::new(s.frame(), &mut far);
    outline.add([-1, -m(1.5), -1], [length + 1, top, depth + 1], K::drystone);
    outline.gable([-2.0, (top - 2) as f64, -3.0], [(length + 2) as f64, top as f64 + rise, (depth + 3) as f64], 0, T::thatch, Op::Fill);
    Node::of(shapes).with_far(Far::of(fx(2.0), far))
}

/** A ring of standing stones round its centre, each a child; one of them fallen. */
fn ring(f: &Moor, s: &Steading, random: &mut Rng) -> Node {
    let mut children = Vec::new();
    let fallen = random.below(s.stones as u32) as i64;
    for i in 0..s.stones {
        let a = (i as f64 + random.next() * 0.3) * 2.0 * std::f64::consts::PI / s.stones as f64;
        let (x, z) = (s.ring[0] as f64 + s.radius as f64 * js::cos(a), s.ring[1] as f64 + s.radius as f64 * js::sin(a));
        let g = f.height(x as i64, z as i64) as f64;
        let (tall, wide) = (m(1.6) as f64 + random.next() * m(1.6) as f64, 2.5 + random.next() * 1.5);
        let kind: Kind = if random.chance(Fx::HALF) { K::lichen.into() } else { F::stone.into() };
        let shape = if i == fallen {
            let (dx, dz) = (js::cos(a + 1.3) * tall, js::sin(a + 1.3) * tall);
            Shape::capsule([x, g + wide, z], [x + dx, g + wide, z + dz], wide, kind)
        } else {
            Shape::lathe([x, g - m(0.5) as f64, z], [x, g + tall, z], move |t| wide * (1.0 - 0.4 * t * t), kind)
        };
        children.push(Child::new(format!("stone {i}"), ANY_SIZE, Node::of(vec![shape])));
    }
    Node::group(children)
}

/** Cairns every 5 m along the path, on either side by turns: three stones stacked. */
fn cairns(f: &Moor, s: &Steading, random: &mut Rng) -> Node {
    let (a, b) = s.path();
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let length = js::hypot(&[dx, dz]);
    let (nx, nz) = (-dz / length, dx / length);
    let mut shapes = Vec::new();
    let mut t = m(3.0) as f64;
    let mut side = 1.0;
    while t < length - m(2.0) as f64 {
        let off = side * m(1.3) as f64;
        let (x, z) = (a[0] + dx * t / length + nx * off, a[1] + dz * t / length + nz * off);
        let g = f.height(x as i64, z as i64) as f64;
        let r = 2.5 + random.next();
        shapes.push(Shape::ball([x, g + r * 0.6, z], r, F::stone));
        shapes.push(Shape::ball([x, g + r * 1.6, z], r * 0.75, K::lichen));
        shapes.push(Shape::ball([x, g + r * 2.3, z], r * 0.5, F::stone));
        t += m(5.0) as f64;
        side = -side;
    }
    Node::of(shapes)
}
