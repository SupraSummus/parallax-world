//! The coast: what stands by the sea west of the ranges, on the lowland's
//! ragged shore. On headlands, lighthouses, white and red, their lamps lit
//! (they glow, so they show at night), each with its keeper's cottage; on
//! the beaches below, the wrecks of ships, half sunk in the sand, their
//! ribs bared, a broken mast and their cargo strewn round them.
//!
//! A part of the valley (valley.rs) that bids for no land: its things are
//! claimed (Claims) from the valley's ground before anything shapes it,
//! where it is a headland or a beach by the sea; the forest keeps off them
//! (`clearing`).

use std::sync::Arc;

use ground::land::clear_of;
use ground::plan::{Claims, Sloped};
use worldgen::frame::{Frame, Part, facing};
use worldgen::generator::{ANY_SIZE, Child, Layer, Node, Op, Point, Population, SOLID, Shape};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, Scope, fx};
use worldgen::scalar::Scalar;
use worldgen::units::m;
use worldgen::world::Find;

use super::Terrain;
use super::badlands;
use super::mountains::{Ranges, SEA};
use super::town::{House, K as T, Style, thing};
use crate::forest::K as F;

kinds! {
    base: badlands::BASE + badlands::KINDS.len() as u8;
    lighthouseRed: 10, ["#b8352b", "#a92f26"];
    beacon: 7, ["#fff6d8", "#fff0c0"], glows;
    hull: 4, ["#4e3b2a", "#5a4431", "#453425"];
    rope: 4, ["#a89466", "#9a875c"];
}

/**
 * Things stand at least APART, claimed (Claims) from spots one in each
 * square CLAIM wide on average, each tried at TRIES places round it; one
 * reaches REACH at most from where it stands along x and z, and up.
 */
const CLAIM: i64 = m(400.0);
const APART: i64 = m(220.0);
const TRIES: usize = 24;
const REACH: [i64; 3] = [m(24.0), m(40.0), m(24.0)];
/** How far round each the forest keeps off. */
const CLEAR: i64 = m(22.0);

/** What stands on the coast. */
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Stands {
    #[default]
    Lighthouse,
    Wreck,
}

/** A thing on the coast: what, where (x, z), its way to the sea, and its seed. */
#[derive(Clone, Copy, Default, Debug)]
struct Claimed {
    stands: Stands,
    at: [i64; 2],
    sea: f64,
    seed: u64,
}

/** The valley's coast. */
pub struct Coast {
    terrain: Arc<Terrain>,
    things: Arc<Claims<Claimed>>,
}

/** How high over the sea column (x, z) is. */
fn over_sea(terrain: &Terrain, x: i64, z: i64) -> Fx {
    terrain.ground(x, z) - SEA.to_fx()
}

/** The quarter turns that face a frame's front (−w) along heading `a`. */
fn toward(a: f64) -> i64 {
    facing((js::cos(a) * 64.0) as i64, (js::sin(a) * 64.0) as i64)
}

/**
 * The way from (x, z) to the sea within `within`, if any: of eight, the
 * first along which the ground is two metres under it, nearest first.
 */
fn seaward(terrain: &Terrain, x: i64, z: i64, within: i64) -> Option<f64> {
    (1..=within / m(10.0)).find_map(|k| {
        (0..8).find_map(|n| {
            let a = n as f64 * std::f64::consts::TAU / 8.0;
            let d = (k * m(10.0)) as f64;
            let (u, w) = (x + (d * js::cos(a)) as i64, z + (d * js::sin(a)) as i64);
            (over_sea(terrain, u, w) < -Fx::m(2.0)).then_some(a)
        })
    })
}

/** What may stand at (x, z), on the lowland west by the sea: a lighthouse on a headland, a wreck on a beach. */
fn claim(terrain: &Terrain, spot: [i64; 2], random: &mut Rng) -> Option<Claimed> {
    for _ in 0..TRIES {
        let (x, z) = (spot[0] - CLAIM / 2 + random.below(CLAIM as u32) as i64, spot[1] - CLAIM / 2 + random.below(CLAIM as u32) as i64);
        let across = terrain.course.across(x, z);
        if -across < Ranges::NEAREST_BEYOND || terrain.ranges.beyond(z, across) < fx(0.9) {
            continue;
        }
        let h = over_sea(terrain, x, z);
        let made = if (Fx::m(3.0)..Fx::m(30.0)).contains(&h) {
            // Even under it, the sea close.
            let even = [(m(4.0), 0), (-m(4.0), 0), (0, m(4.0)), (0, -m(4.0))]
                .iter()
                .all(|(dx, dz)| (over_sea(terrain, x + dx, z + dz) - h).abs() < Fx::m(1.5));
            seaward(terrain, x, z, m(160.0)).filter(|_| even).map(|sea| (Stands::Lighthouse, sea))
        } else if (Fx::m(0.2)..Fx::m(1.2)).contains(&h) {
            seaward(terrain, x, z, m(30.0)).map(|sea| (Stands::Wreck, sea))
        } else {
            None
        };
        if let Some((stands, sea)) = made {
            return Some(Claimed { stands, at: [x, z], sea, seed: random.next_u64() });
        }
    }
    None
}

impl Coast {
    pub fn new(scope: &Scope, terrain: Arc<Terrain>) -> Coast {
        let things = {
            let terrain = terrain.clone();
            Arc::new(Claims::new("coast", scope.seed("things"), CLAIM, APART, move |spot, random| {
                claim(&terrain, spot, random).map(|c| (c.at, c))
            }))
        };
        Coast { terrain, things }
    }

    /** The `stands` nearest the start of the path, on `ground`. */
    fn first(&self, stands: Stands, ground: &Sloped) -> [i64; 3] {
        let start = self.terrain.path.x(0).round_units().0;
        let found = self.things.nearest(start, 0, m(64_000.0), |c| c.stands == stands);
        let ([x, z], _) = found.unwrap_or_else(|| panic!("no {stands:?} within 64 km of the start"));
        [x, ground.at(x, z), z]
    }

    /** The places it names: the lighthouse and the wreck nearest the start, on `ground`. */
    pub fn places(self: &Arc<Self>, ground: &Arc<Sloped>) -> Vec<(&'static str, Find)> {
        let named = |name, stands| -> (&'static str, Find) {
            let (coast, ground) = (self.clone(), ground.clone());
            (name, Box::new(move || coast.first(stands, &ground)))
        };
        vec![named("lighthouse", Stands::Lighthouse), named("wreck", Stands::Wreck)]
    }

    /** How much of a column the forest keeps round what stands on the coast (`clear_of`). */
    pub fn clearing(&self) -> Scalar {
        clear_of(self.things.clone(), CLEAR)
    }

    /** Its lighthouses and wrecks, standing on `ground`. */
    pub fn layers(self: Arc<Self>, ground: &Arc<Sloped>) -> Vec<Layer> {
        let ground = ground.clone();
        let built = Population::new("coast", 1.6, m(24.0)).reach(REACH).size(ANY_SIZE).at(
            move |b| self.things.over(b).into_iter().map(|([x, z], c)| ([x as f64, ground.at(x, z) as f64, z as f64], c)).collect(),
            |c, anchor| {
                let mut random = Rng::new(c.seed);
                Some(match c.stands {
                    Stands::Lighthouse => lighthouse(anchor.at, c.sea, &mut random),
                    Stands::Wreck => wreck(anchor.at, c.sea, &mut random),
                })
            },
        );
        vec![Layer::Population(built)]
    }
}

/**
 * A lighthouse in its own frame, about its axis (v), its door to land
 * (−w): a round tower tapering up, white with red bands, on a footing of
 * stone; a gallery round its top, railed, the lantern over it, its lamp
 * lit, under a dome; its keeper's cottage landward, its front to the tower.
 */
fn lighthouse([x, y, z]: Point, sea: f64, random: &mut Rng) -> Node {
    let frame = Frame::centred([x as i64, y as i64, z as i64], [0; 3], toward(sea + std::f64::consts::PI));
    let (tall, r0) = (m(18.0) as f64 + random.next() * m(8.0) as f64, m(2.6) as f64 + random.next() * m(0.6) as f64);
    let radius = move |t: f64| r0 * (1.0 - 0.32 * t);
    let up = |v: f64| [0.0, v, 0.0];
    let mut tower = Vec::new();
    let mut part = Part::new(frame, &mut tower);
    part.lathe(up(-m(3.0) as f64), up(m(1.0) as f64), r0 + m(0.8) as f64, T::churchStone, SOLID, Op::Fill);
    part.lathe(up(0.0), up(tall), radius, T::plasterWhite, SOLID, Op::Fill);
    // Red bands, the tower's own shape painted over it.
    let bands = 3 + random.below(2) as usize;
    for n in 0..bands {
        let (t0, t1) = ((2 * n + 1) as f64 / (2 * bands + 1) as f64, (2 * n + 2) as f64 / (2 * bands + 1) as f64);
        part.lathe(up(tall * t0), up(tall * t1), move |t: f64| radius(t0 + (t1 - t0) * t), K::lighthouseRed, SOLID, Op::Paint);
    }
    // Its door, set into the wall, as a windmill's is.
    let front = -(r0 as i64);
    part.cut([-5, 0, front - 3], [5, m(2.1), front + 5]);
    part.paint([-5, 0, front + 5], [5, m(2.1), front + 6], T::door);
    // The gallery, a railing round it; the lantern, its glazing bars; its dome.
    let (top, rt) = (tall + 3.0, radius(1.0));
    let mut head = Vec::new();
    let mut part = Part::new(frame, &mut head);
    part.lathe(up(tall), up(top), rt + m(0.9) as f64, T::iron, SOLID, Op::Fill);
    part.lathe(up(top), up(top + m(2.4) as f64), rt * 0.7, K::beacon, SOLID, Op::Fill);
    part.lathe(
        up(top + m(2.4) as f64),
        up(top + m(3.6) as f64),
        move |t: f64| (rt * 0.85) * (1.0 - t) + 1.0,
        K::lighthouseRed,
        SOLID,
        Op::Fill,
    );
    part.capsule(up(top + m(3.5) as f64), up(top + m(4.6) as f64), 0.7, T::iron, Op::Fill);
    let rail = top + m(1.0) as f64;
    for (n, r, high) in [(8, rt * 0.72, m(2.4) as f64), (16, rt + m(0.8) as f64, m(1.0) as f64)] {
        for k in 0..n {
            let a = k as f64 * std::f64::consts::TAU / n as f64;
            let (u, w) = (r * js::cos(a), r * js::sin(a));
            part.capsule([u, top, w], [u, top + high, w], 0.6, T::iron, Op::Fill);
        }
    }
    part.lathe(up(rail), up(rail + 1.0), rt + m(0.9) as f64, T::iron, SOLID, Op::Fill);
    part.lathe(up(rail), up(rail + 1.0), rt + m(0.7) as f64, 0, SOLID, Op::Carve);
    // The keeper's cottage, landward, turned to face the tower.
    let (length, depth) = (m(8.0) + random.below(m(2.0) as u32) as i64, m(6.0));
    let far = r0 as i64 + m(4.0) + depth / 2;
    let mut cottage = Vec::new();
    let house = frame.then(Frame::centred([0, 1, -far], [length, 0, depth], 2));
    House::new(length, depth, 1, Style::of(random.next_u64())).build(&mut Part::new(house, &mut cottage), random);
    Node::group(vec![
        Child::new("tower", ANY_SIZE, thing(tower)),
        Child::new("lantern", ANY_SIZE, Node::of(head)),
        Child::new("cottage", ANY_SIZE, thing(cottage)),
    ])
}

/**
 * A wreck: a ship's hull in its own frame, its keel along the shore (u),
 * its side to the sea (−w), half sunk in the sand and listing, open above;
 * its planking gone in places down its seaward side, baring its ribs; its
 * mast broken off and lying by it; barrels and crates strewn round it,
 * towards the sea.
 */
fn wreck([x, y, z]: Point, sea: f64, random: &mut Rng) -> Node {
    let (long, beam, deep) = (m(9.0) + random.below(m(6.0) as u32) as i64, m(2.4) + random.below(m(1.0) as u32) as i64, m(2.6));
    let (dx, dz) = (js::cos(sea), js::sin(sea));
    let frame = Frame::centred([x as i64, y as i64, z as i64], [2 * long, 0, 2 * beam], toward(sea));
    let mut hull = Vec::new();
    let mut part = Part::new(frame, &mut hull);
    // Its middle a little under the sand, listing to one side.
    let list = (random.next() - 0.5) * m(1.4) as f64;
    let c = [long as f64, -(deep as f64) * 0.25, beam as f64 + list];
    let radii = [long as f64, deep as f64, beam as f64];
    part.ellipsoid(c, radii, K::hull, Op::Fill);
    part.ellipsoid(c, radii.map(|r| r - m(0.3) as f64), K::hull, Op::Carve);
    let deck = (c[1] + deep as f64 * 0.45) as i64;
    part.cut([-1, deck, -1], [2 * long + 1, deck + deep + 1, 2 * beam + 1]);
    // Planking gone between its ribs, down its seaward side.
    let gaps = 3 + random.below(4) as i64;
    for n in 0..gaps {
        let u = long / 5 + (8 * long / 5) * (2 * n + 1) / (2 * gaps);
        part.cut([u - m(0.5), (c[1] - deep as f64 * 0.1) as i64, -1], [u + m(0.5), deck, beam]);
    }
    // Its mast, broken, lying from beside it toward the sea; rope trailing from it.
    let reach = (beam + m(1.0)) as f64;
    let foot = [x + dx * reach, y + m(0.25) as f64, z + dz * reach];
    let mast_long = m(6.0) as f64 + random.next() * m(5.0) as f64;
    let skew = (random.next() - 0.5) * 1.2;
    let tip = [foot[0] + mast_long * js::cos(sea + skew), y + m(0.2) as f64, foot[2] + mast_long * js::sin(sea + skew)];
    let mut strewn = vec![Shape::capsule(foot, tip, move |t: f64| m(0.22) as f64 * (1.0 - 0.4 * t), F::deadwood)];
    strewn.push(Shape::capsule(tip, [tip[0] + dz * m(1.5) as f64, y, tip[2] - dx * m(1.5) as f64], 0.4, K::rope));
    // Cargo: barrels on their sides and crates, between it and the sea.
    for _ in 0..3 + random.below(5) {
        let (d, a) = (m(2.0) as f64 + random.next() * m(8.0) as f64, sea + (random.next() - 0.5) * 2.0);
        let (u, w) = (x + d * js::cos(a), z + d * js::sin(a));
        if random.chance(Fx::HALF) {
            let r = m(0.3) as f64;
            let turn = random.next() * std::f64::consts::PI;
            let (ex, ez) = (js::cos(turn) * m(0.45) as f64, js::sin(turn) * m(0.45) as f64);
            strewn.push(Shape::lathe([u - ex, y + r - 1.0, w - ez], [u + ex, y + r - 1.0, w + ez], r, T::planks));
        } else {
            let half = m(0.3) as f64 + random.next() * m(0.15) as f64;
            strewn.push(Shape::cuboid([u - half, y - 2.0, w - half], [u + half, y + 2.0 * half - 2.0, w + half], T::planks));
        }
    }
    Node::group(vec![Child::new("hull", ANY_SIZE, thing(hull)), Child::new("strewn", ANY_SIZE, Node::of(strewn))])
}
