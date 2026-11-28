//! Badlands: patches of the upland east of the ranges, kilometres across,
//! where the forest and the moor give way to bare red rock. Its slopes
//! are banded by height, ochre, red and pale, as the strata run; on its
//! flats, red sand and dry scrub, a bleached dead tree here and there.
//! Out of it stand rock formations, claimed apart: clusters of hoodoos,
//! each a crumbling pillar under its cap stone, buttes walled in strata
//! over skirts of scree, and arches.
//!
//! A part of the valley (valley.rs), from position alone: it bids for the
//! land where its patches lie, past the ranges (`Ranges::beyond`), and
//! draws where it owns it; its formations stand only where it surely does.

use std::sync::Arc;

use ground::column::{Column, lowest_around};
use ground::land::{Columns, of_columns};
use ground::plan::{Claims, Plan, Sloped};
use ground::{FieldAbout, PopulationOn, standing};
use worldgen::generator::{ANY_SIZE, Child, Kind, Layer, Node, Op, Point, Population, Shape};
use worldgen::js;
use worldgen::kinds;
use worldgen::land::owns;
use worldgen::noise::{Fbm, Fx, Rng, Scope, ValueNoise, fx, hash, smoothstep};
use worldgen::points::Points;
use worldgen::scalar::Scalar;
use worldgen::units::{Units, m};
use worldgen::world::Find;

use super::Terrain;
use super::caves;
use super::course::COURSE_SLOPE;
use super::mountains::Ranges;
use crate::SURFACE;

kinds! {
    base: caves::BASE + caves::KINDS.len() as u8;
    redRock: 3, ["#b5532e", "#a84a28", "#bf5c35"];
    ochreRock: 3, ["#d08a45", "#c47e3c", "#d99550"];
    paleRock: 3, ["#e0c69c", "#d6ba8f", "#e8d2ad"];
    capRock: 3, ["#6e5546", "#7a6151", "#634c3f"];
    redSand: 2, ["#c8784b", "#bd6d42", "#d08454"];
    scrub: 5, ["#7d7a45", "#8a8650", "#6f6c3c"], walk_through;
    bleached: 4, ["#d9d2c3", "#cfc6b5", "#e2dccf"];
}

/** Its patches: a noise this wide, and how ragged their edges are. */
const PATCH: i64 = m(2400.0);
/** How far its bid leads or trails the rest, at most. */
const MOST: Fx = fx(3.0);
/**
 * Formations stand at least APART, claimed from spots one in each square
 * CLAIM wide on average, where it surely owns the land (`core`); each
 * reaches REACH at most from where it stands, along x and z (a butte's
 * scree, 1.45 of its widest, 24 m), and up.
 */
const CLAIM: i64 = m(150.0);
const APART: i64 = m(72.0);
const REACH: [i64; 3] = [m(36.0), m(60.0), m(36.0)];
/** How thick its strata are. */
const STRATUM: i64 = m(2.5);
/** The core of a patch: what the bid is all ahead from, and formations stand in. */
const CORE: Fx = fx(0.75);

/** Where the badlands' patches lie: from position alone. */
struct Patches {
    terrain: Arc<Terrain>,
    patches: ValueNoise<PATCH>,
    ragged: Fbm<{ m(140.0) }, 2>,
}

impl Patches {
    /** How much column (x, z) is in a patch: 0 off them, 1 in their cores. */
    fn core(&self, x: i64, z: i64) -> Fx {
        let across = self.terrain.course.across(x, z);
        if across < Ranges::NEAREST_BEYOND {
            return Fx::ZERO;
        }
        let beyond = self.terrain.ranges.beyond(z, across);
        let patch = self.patches.at(x, z) + (self.ragged.at(x, z) - Fx::HALF) / 5;
        smoothstep(fx(0.5), fx(0.62), patch) * beyond
    }

    /** The formation claimed from `spot`, if any: one of the kinds, by weight, where the patch is all its own round it. */
    fn claim(&self, spot: [i64; 2], random: &mut Rng) -> Option<Formation> {
        let at = [spot[0] - CLAIM / 2 + random.below(CLAIM as u32) as i64, spot[1] - CLAIM / 2 + random.below(CLAIM as u32) as i64];
        let around = [(0, 0), (m(30.0), 0), (-m(30.0), 0), (0, m(30.0)), (0, -m(30.0))];
        if !around.iter().all(|(dx, dz)| self.core(at[0] + dx, at[1] + dz) >= CORE) {
            return None;
        }
        let shape = match random.below(8) {
            0..=4 => Formed::Hoodoos,
            5..=6 => Formed::Butte,
            _ => Formed::Arch,
        };
        Some(Formation { shape, at, seed: random.next_u64() })
    }
}

/** The badlands of a valley: where its patches lie, its formations. */
pub struct Badlands {
    patches: Arc<Patches>,
    formations: Claims<Formation>,
    /** Its strata, the same in its slopes and its formations: a rock's band at a height is the slope's there. */
    strata: u64,
    plants: u64,
}

/** What a formation is. */
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Formed {
    #[default]
    Hoodoos,
    Butte,
    Arch,
}

/** A formation where it stands, and its seed. */
#[derive(Clone, Copy, Default, Debug)]
struct Formation {
    shape: Formed,
    at: [i64; 2],
    seed: u64,
}

impl Badlands {
    pub fn new(scope: &Scope, terrain: Arc<Terrain>) -> Badlands {
        let patches =
            Arc::new(Patches { terrain, patches: ValueNoise::new(scope.seed("patches")), ragged: Fbm::new(scope.seed("ragged")) });
        let formations = {
            let patches = patches.clone();
            Claims::new("badlands' formations", scope.seed("formations"), CLAIM, APART, move |spot, random| {
                patches.claim(spot, random).map(|f| (f.at, f))
            })
        };
        Badlands { patches, formations, strata: scope.seed("strata"), plants: scope.seed("plants") }
    }

    /** Whether any column `min` to `max` may be east past the ranges: the least across them the course is, by its slope. */
    fn may_be(&self, min: [i64; 2], max: [i64; 2]) -> bool {
        let mid = (min[1] + max[1]) / 2;
        let bend = fx(COURSE_SLOPE) * Units((max[1] - min[1]) / 2 + 1);
        Units(max[0]).to_fx() - self.patches.terrain.course.x(mid) + bend >= Ranges::NEAREST_BEYOND
    }

    /** Its bid: all ahead in its patches' cores, as far behind off them; nothing but there. */
    pub fn bid(self: &Arc<Self>) -> Scalar {
        let (at, over) = (self.clone(), self.clone());
        of_columns(
            move |x, z| (at.patches.core(x, z) / CORE).min(Fx::ONE) * MOST * 2 - MOST,
            move |min, max| (-MOST, if over.may_be(min, max) { MOST } else { -MOST }),
        )
    }

    /** The formation nearest the start of the path: where it stands, on `ground`. */
    fn first(&self, ground: &Sloped) -> [i64; 3] {
        let start = self.patches.terrain.path.x(0).round_units().0;
        let ([x, z], _) = self.formations.nearest(start, 0, m(64_000.0), |_| true).expect("badlands within 64 km of the start");
        [x, ground.at(x, z), z]
    }

    /** The places it names: the formation nearest the start, on `ground`. */
    pub fn places(self: &Arc<Self>, ground: &Arc<Sloped>) -> Vec<(&'static str, Find)> {
        let (badlands, ground) = (self.clone(), ground.clone());
        vec![("badlands", Box::new(move || badlands.first(&ground)))]
    }

    /** Its ground, its scrub and dead trees, its formations; where it owns the land, `hold`. */
    pub fn layers(self: Arc<Self>, ground: &Arc<Sloped>, hold: Scalar) -> Vec<Layer> {
        layers(self, ground.clone(), hold)
    }
}

/** The stratum at height `h`: red, ochre, red, pale by turns, a little thicker or thinner by `seed`. */
fn stratum(h: i64, seed: u64) -> Kind {
    let n = h.div_euclid(STRATUM);
    match (n + i64::from(hash(seed, n.div_euclid(4), 0, 0) % 4)).rem_euclid(4) {
        0 | 2 => K::redRock.into(),
        1 => K::ochreRock.into(),
        _ => K::paleRock.into(),
    }
}

fn layers(badlands: Arc<Badlands>, ground: Arc<Sloped>, hold: Scalar) -> Vec<Layer> {
    let strata = badlands.strata;
    // What each column is: sand on its flats, on its slopes the stratum there; read dithered where sites are further apart.
    let lies = {
        let ground = ground.clone();
        Arc::new(Plan::dithered("badlands' ground", m(1.0), move |x, z| {
            let h = ground.at(x, z);
            let steep = [(m(1.0), 0), (-m(1.0), 0), (0, m(1.0)), (0, -m(1.0))]
                .into_iter()
                .map(|(dx, dz)| (ground.at(x + dx, z + dz) - h).abs())
                .max()
                .unwrap_or(0);
            if steep * 10 >= m(1.0) * 6 { stratum(h, strata) } else { K::redSand.into() }
        }))
    };
    let surface = {
        let (ground, owned, lies) = (ground.clone(), hold.clone(), lies.clone());
        let on = ground.clone();
        ground::columns("badlands", 0.82, Op::Paint, move |x, z| {
            if !owns(owned.column(x, z)) {
                return Vec::new();
            }
            let h = ground.at(x, z);
            let lowest = lowest_around(|x, z| ground.at(x, z), x, z);
            Column::new(h).surface(lies.at(x, z), SURFACE).sides(K::redRock, lowest, m(1.5)).spans()
        })
        .about(on, SURFACE, 1)
        .within(hold.clone(), 0)
    };
    // Scrub: one bush in 30 squares 2 m wide, a dead tree in 400, by its share; in leaves 4 m wide.
    let share = hold.clone();
    let points = Points::new("badlands' plants", badlands.plants, &[0, 2], m(4.0), m(16.0), 2, move |[x, _, z]| {
        let s = share.column(x, z);
        [fx(4.0 / 30.0) * s, fx(4.0 / 400.0) * s, Fx::ZERO, Fx::ZERO]
    });
    let plants = {
        let (ground, plants, owned) = (ground.clone(), badlands.plants, hold.clone());
        Population::new("badlands' plants", 3.0, m(2.0))
            .reach([m(4.0), m(7.0), m(4.0)])
            .size(m(2.0))
            .on(ground.clone())
            .within(hold.clone(), m(4.0))
            .at(
                move |b| standing(&points, b, |x, z| ground.at(x, z)),
                move |spot, anchor| {
                    let [x, y, z] = anchor.at;
                    if !owns(owned.column(x as i64, z as i64)) {
                        return None;
                    }
                    let mut random = anchor.random(plants);
                    Some(if spot.mark == 0 { bush([x, y, z], &mut random) } else { dead_tree([x, y, z], &mut random) })
                },
            )
    };
    let formations = {
        let (anchor, ground) = (badlands.clone(), ground.clone());
        let strata = badlands.strata;
        Population::new("badlands' formations", 1.6, m(32.0)).reach(REACH).size(ANY_SIZE).within(hold, REACH[0]).at(
            move |b| anchor.formations.over(b).into_iter().map(|([x, z], f)| ([x as f64, ground.at(x, z) as f64, z as f64], f)).collect(),
            move |f, anchor| {
                let mut random = Rng::new(f.seed);
                let at = anchor.at;
                Some(match f.shape {
                    Formed::Hoodoos => hoodoos(at, strata, &mut random),
                    Formed::Butte => butte(at, strata, &mut random),
                    Formed::Arch => arch(at, &mut random),
                })
            },
        )
    };
    vec![Layer::Field(surface), Layer::Population(plants), Layer::Population(formations)]
}

/** A bush of dry scrub: a few low clumps. */
fn bush([x, y, z]: Point, random: &mut Rng) -> Node {
    let shapes = (0..2 + random.below(3))
        .map(|_| {
            let r = m(0.25) as f64 + random.next() * m(0.35) as f64;
            let at = [x + (random.next() - 0.5) * m(0.8) as f64, y + r * 0.4, z + (random.next() - 0.5) * m(0.8) as f64];
            Shape::ellipsoid(at, [r, r * 0.7, r], K::scrub)
        })
        .collect();
    Node::of(shapes)
}

/** A dead tree, bleached: a leaning trunk and a few bare limbs off its top half. */
fn dead_tree([x, y, z]: Point, random: &mut Rng) -> Node {
    let (tall, r) = (m(3.0) as f64 + random.next() * m(2.5) as f64, m(0.12) as f64 + random.next() * m(0.08) as f64);
    let lean = [(random.next() - 0.5) * 0.5, (random.next() - 0.5) * 0.5];
    let top = [x + lean[0] * tall, y + tall, z + lean[1] * tall];
    let mut shapes = vec![Shape::capsule([x, y - m(0.5) as f64, z], top, move |t: f64| r * (1.0 - 0.6 * t), K::bleached)];
    for _ in 0..3 + random.below(3) {
        let t = 0.45 + random.next() * 0.5;
        let from = [x + (top[0] - x) * t, y + tall * t, z + (top[2] - z) * t];
        let a = random.next() * std::f64::consts::TAU;
        let long = m(0.8) as f64 + random.next() * m(1.2) as f64;
        let to = [from[0] + long * js::cos(a), from[1] + long * (0.3 + random.next() * 0.6), from[2] + long * js::sin(a)];
        shapes.push(Shape::capsule(from, to, move |t: f64| r * 0.5 * (1.0 - 0.7 * t), K::bleached));
    }
    Node::of(shapes)
}

/**
 * A pillar of rock from below `foot` up `tall`, its radius `radius(t)` a
 * fraction t of the way up, in the strata of `seed`: red rock all through,
 * the other strata painted over it as thick as they are, the same shape;
 * its top `cap` thick cap rock, harder than the rest.
 */
fn banded(
    shapes: &mut Vec<Shape>,
    foot: Point,
    tall: f64,
    cap: f64,
    radius: impl Fn(f64) -> f64 + Clone + Send + Sync + 'static,
    seed: u64,
) {
    let (bottom, top) = (foot[1] - m(4.0) as f64, foot[1] + tall);
    let mut band = |from: f64, to: f64, kind: Kind, op: fn(Shape) -> Shape| {
        let (t0, t1, r) = ((from - bottom) / (top - bottom), (to - bottom) / (top - bottom), radius.clone());
        shapes.push(op(Shape::lathe([foot[0], from, foot[2]], [foot[0], to, foot[2]], move |t: f64| r(t0 + (t1 - t0) * t), kind)));
    };
    band(bottom, top, K::redRock.into(), |s| s);
    let mut y = (bottom as i64).div_euclid(STRATUM) * STRATUM;
    while (y as f64) < top - cap {
        let kind = stratum(y, seed);
        if kind != K::redRock.into() {
            band((y as f64).max(bottom), ((y + STRATUM) as f64).min(top - cap), kind, Shape::paint);
        }
        y += STRATUM;
    }
    if cap > 0.0 {
        band(top - cap, top, K::capRock.into(), Shape::paint);
    }
}

/** Hoodoos: a cluster of crumbling pillars, each under its cap stone. */
fn hoodoos(at: Point, strata: u64, random: &mut Rng) -> Node {
    let children = (0..3 + random.below(6))
        .map(|n| {
            let mut shapes = Vec::new();
            let (a, d) = (random.next() * std::f64::consts::TAU, random.next().sqrt() * m(13.0) as f64);
            let foot = [at[0] + d * js::cos(a), at[1], at[2] + d * js::sin(a)];
            let (r0, tall) = (m(1.0) as f64 + random.next() * m(1.4) as f64, m(6.0) as f64 + random.next() * m(13.0) as f64);
            let phase = random.next() * 6.0;
            banded(&mut shapes, foot, tall, 0.0, move |t: f64| r0 * (1.15 - 0.4 * t) * (1.0 + 0.16 * js::sin(t * 13.0 + phase)), strata);
            let cap = r0 * (1.1 + random.next() * 0.4);
            shapes.push(Shape::ellipsoid([foot[0], foot[1] + tall + cap * 0.25, foot[2]], [cap, cap * 0.45, cap], K::capRock));
            Child::new(format!("hoodoo {n}"), ANY_SIZE, Node::of(shapes))
        })
        .collect();
    Node::group(children)
}

/** A butte: walls of strata, a little in from its foot, over a skirt of scree; flat on top under its cap rock. */
fn butte(at: Point, strata: u64, random: &mut Rng) -> Node {
    let (r0, tall) = (m(12.0) as f64 + random.next() * m(12.0) as f64, m(16.0) as f64 + random.next() * m(22.0) as f64);
    let phase = random.next() * 6.0;
    let mut shapes = Vec::new();
    banded(&mut shapes, at, tall, m(1.5) as f64, move |t: f64| r0 * (1.0 - 0.1 * t) * (1.0 + 0.05 * js::sin(t * 7.0 + phase)), strata);
    // Scree fallen round its foot.
    shapes.push(Shape::lathe(
        [at[0], at[1] - m(3.0) as f64, at[2]],
        [at[0], at[1] + tall * 0.3, at[2]],
        move |t: f64| r0 * (1.45 - 0.5 * t),
        K::ochreRock,
    ));
    Node::of(shapes)
}

/** An arch: a ring of rock standing on end, its lower part in the ground, the sky through it. */
fn arch(at: Point, random: &mut Rng) -> Node {
    let (outer, thick) = (m(8.0) as f64 + random.next() * m(6.0) as f64, m(2.5) as f64 + random.next() * m(2.0) as f64);
    let inner = outer - m(2.5) as f64 - random.next() * m(1.5) as f64;
    let a = random.next() * std::f64::consts::PI;
    let (dx, dz) = (js::cos(a) * thick / 2.0, js::sin(a) * thick / 2.0);
    let centre = [at[0], at[1] + outer * 0.2, at[2]];
    let (p, q) = ([centre[0] - dx, centre[1], centre[2] - dz], [centre[0] + dx, centre[1], centre[2] + dz]);
    let kind = if random.chance(Fx::HALF) { K::ochreRock } else { K::redRock };
    Node::of(vec![Shape::lathe(p, q, outer, kind), Shape::lathe(p, q, inner, kind).carve()])
}
