//! The moor: rolling heath of heather and grass, a stream and pools in
//! its hollows, scattered boulders, and steadings: a stone hut with a
//! thatched roof, a ring of standing stones, and a path of cairns between
//! them. Written from the engine's public API alone (docs/generators.md).

use std::rc::Rc;

use worldgen::World;
use worldgen::column::Column;
use worldgen::frame::{Frame, Part, facing, walls};
use worldgen::generator::{ANY_SIZE, Child, Far, Field, Generator, Layer, Node, Op, Population, Shape, Span};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, fixed, fx, hash64, smoothstep, stream};
use worldgen::plan::Plan;
use worldgen::units::{Units, m};

kinds! {
    base: 0;
    heather: 5, ["#6b3a5e", "#7a4569", "#5e3352", "#84506f"];
    grass: 1, ["#8a9a4a", "#7d8c42", "#96a556"];
    bog: 1, ["#5a7a3a", "#4f6e33"];
    peat: 2, ["#3b2a1e", "#33241a", "#423022"];
    gravel: 2, ["#9a8f7a", "#8c8270", "#a59a84"];
    granite: 3, ["#8a8680", "#7c7872", "#96928a"];
    lichen: 3, ["#9a9a7a", "#a6a682", "#8e8e70"];
    drystone: 3, ["#6f6a62", "#7a756c", "#655f58"];
    thatch: 9, ["#a8874a", "#9a7a40", "#b4945a"];
    timber: 4, ["#4a3424", "#563d2a"];
    water: 13, ["#3f6a8a", "#4a7898"];
}

/** How far around the camera level 0's grid must hold the moor, and the heights it has anything at (lattice units). */
const REACH: f64 = 352.0;
const HEIGHTS: (i64, i64) = (m(-24.0), m(32.0));

/** Water stands in the hollows up to this height. */
const WATER: Units = Units::m(-4.5);
/** Plan scales: the stream's bends, the ground's bumps, the steadings' regions (one at most in each). */
const STREAM_SCALE: i64 = m(32.0);
const GROUND_SCALE: i64 = 8;
const REGION: i64 = m(160.0);
/** A steading's ring is this far inside its region, so the cell that owns it is centred in the region too. */
const MARGIN: i64 = m(50.0);
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
const WALL: i64 = 3;

/** The moor's relief and noise, from its seed alone. */
struct Relief {
    hills: Fbm<{ m(240.0) }, 4>,
    bumps: ValueNoise<{ m(10.0) }>,
    patches: Fbm<{ m(30.0) }, 2>,
    phase: f64,
}

impl Relief {
    /** The heath's height at (x, z), with its fraction: hills tens of metres apart, bumps of a few. */
    fn heath(&self, x: i64, z: i64) -> Fx {
        (self.hills.fx(x, z) - Fx::HALF) * Units::m(28.0) + (self.bumps.fx(x, z) - Fx::HALF) * Units::m(0.8)
    }

    /** The stream's centre line at z, with its fraction: winding along z west of the origin. */
    fn stream_x(&self, z: i64) -> Fx {
        let zm = z as f64 / m(1.0) as f64;
        fx(m(-40.0) as f64 + m(1.0) as f64 * (12.0 * js::sin(zm / 37.0 + self.phase) + 4.0 * js::sin(zm / 11.0 + 2.0 * self.phase)))
    }
}

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
}

/** Distance from (x, z) to the segment from a to b. */
fn to_segment(x: f64, z: f64, a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 == 0.0 { 0.0 } else { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) };
    js::hypot(&[x - a[0] - t * dx, z - a[1] - t * dz])
}

struct Moor {
    relief: Rc<Relief>,
    stream: Rc<Plan<Fx>>,
    steadings: Rc<Plan<Option<Steading>>>,
    ground: Plan<Units>,
}

/** The region of column (x, z): REGION wide, region (0, 0) around the origin. */
fn region(x: i64, z: i64) -> (i64, i64) {
    ((x + REGION / 2).div_euclid(REGION), (z + REGION / 2).div_euclid(REGION))
}

impl Moor {
    fn steading(&self, x: i64, z: i64) -> Option<Steading> {
        let (i, k) = region(x, z);
        self.steadings.at(i, k)
    }

    /** How far column (x, z) is from the stream's centre line; and its half width there. */
    fn stream_distance(&self, x: i64, z: i64) -> (Units, Units) {
        ((Units(x).to_fx() - self.stream.at(0, z)).abs().floor_units(), Units::m(1.0) + Units::m(0.6) * z.rem_euclid(97) / 97)
    }

    /** Whether column (x, z) is on the path of its region's steading. */
    fn on_path(&self, x: i64, z: i64) -> bool {
        self.steading(x, z).is_some_and(|s| {
            let (a, b) = s.path();
            to_segment(x as f64, z as f64, a, b) < m(0.7) as f64
        })
    }
}

/** The steading of region (i, k), if it has one: its ring placed MARGIN inside it, the hut 22 to 30 m off it, both off the stream and out of the water. */
fn steading_of(seed: u64, relief: &Relief, i: i64, k: i64) -> Option<Steading> {
    let mut random = Rng::new(hash64(seed, i, k, 0));
    if (i, k) != (0, 0) && !random.chance(fx(0.6)) {
        return None;
    }
    let inner = (REGION - 2 * MARGIN) as u32;
    let (x0, z0) = (i * REGION - REGION / 2 + MARGIN, k * REGION - REGION / 2 + MARGIN);
    let ring = if (i, k) == (0, 0) { [m(8.0), m(10.0)] } else { [x0 + random.below(inner) as i64, z0 + random.below(inner) as i64] };
    // The first steading's hut east of its ring, away from the stream.
    let turn = if (i, k) == (0, 0) { 0.6 * random.next() - 0.3 } else { random.next() };
    let angle = turn * 2.0 * std::f64::consts::PI;
    let from = m(22.0) as f64 + random.next() * m(8.0) as f64;
    let centre = [ring[0] + (from * js::cos(angle)) as i64, ring[1] + (from * js::sin(angle)) as i64];
    let (length, depth) = (m(5.0) + random.below(m(1.5) as u32) as i64, m(4.0) + random.below(m(1.0) as u32) as i64);
    // Its front faces the ring.
    let turns = facing(ring[0] - centre[0], ring[1] - centre[1]);
    // A hut's level is the mean of the heath under it, as coarse as the hut.
    let mean = |c: [i64; 2]| {
        let around = (-1..=1).flat_map(|a| (-1..=1).map(move |b| (a, b)));
        Fx::mean(around.map(|(a, b)| relief.heath(c[0] + a * m(4.0), c[1] + b * m(4.0)))).round_units()
    };
    let level = mean(centre);
    let off_stream = |c: [i64; 2]| (Units(c[0]).to_fx() - relief.stream_x(c[1])).abs() > Units::m(14.0).to_fx();
    if level <= WATER + Units::m(1.0) || !off_stream(centre) || !off_stream(ring) {
        return None;
    }
    let ring_ground = mean(ring);
    let radius = m(7.0) + random.below(m(3.0) as u32) as i64;
    let stones = 9 + random.below(5) as i64;
    Some(Steading { ring, ring_ground, radius, stones, centre, level, length, depth, turns, seed: random.next_u64() })
}

/** The moor of `seed`; the camera starts behind the first steading's hut, looking past it to the ring. */
pub fn moor(seed: u64) -> World {
    let relief = Rc::new(Relief {
        hills: Fbm::new(stream(seed, 1)),
        bumps: ValueNoise::new(stream(seed, 2)),
        patches: Fbm::new(stream(seed, 3)),
        phase: (hash64(stream(seed, 4), 0, 0, 0) % 1000) as f64 / 100.0,
    });
    let stream_plan = {
        let relief = relief.clone();
        Rc::new(Plan::new("stream", STREAM_SCALE, move |_, z| relief.stream_x(z)))
    };
    let steadings = {
        let (relief, seed) = (relief.clone(), stream(seed, 5));
        Rc::new(Plan::new("steadings", REGION, move |i, k| steading_of(seed, &relief, i, k)))
    };
    // The heath, levelled under each hut.
    let ground = {
        let (relief, steadings) = (relief.clone(), steadings.clone());
        Plan::new("ground", GROUND_SCALE, move |x, z| {
            let heath = relief.heath(x, z);
            let (i, k) = region(x, z);
            let h = match steadings.at(i, k) {
                Some(s) => {
                    let c = s.centre;
                    let d = Units(((x - c[0]).abs().max((z - c[1]).abs()) - HUT_HALF).max(0));
                    let weight = Fx::ONE - smoothstep(Fx::ZERO, TERRACE.to_fx(), d.to_fx());
                    heath + (s.level.to_fx() - heath) * weight
                }
                None => heath,
            };
            h.round_units()
        })
    };
    let f = Rc::new(Moor { relief, stream: stream_plan, steadings, ground });
    let spawn = {
        // Behind the first hut, or over the origin if its region has none.
        let [x, _, z] = f.steading(0, 0).map_or([0; 3], |s| s.frame().site([s.length / 2, 0, s.depth + m(6.0)]));
        let y = (f.ground.at(x, z).max(WATER) + Units::m(2.0)).0;
        [x, y + ((x + y + z) & 1), z]
    };
    let layers = vec![
        Layer::Field(terrain(f.clone())),
        Layer::Field(stream_bed(f.clone())),
        Layer::Field(water(f.clone())),
        Layer::Population(boulders(f.clone())),
        Layer::Population(steadings_population(f)),
    ];
    World { generator: Generator { kinds: KINDS.to_vec(), seed: stream(seed, 6), layers }, spawn, reach: REACH, heights: HEIGHTS }
}

/** The ground: peat under heather, grass or bog, a gravel path to each ring; heather and grass standing on it as media. */
fn terrain(f: Rc<Moor>) -> Field {
    Field::columns("terrain", 0.0, Op::Paint, move |x, z, spacing| {
        let h = f.ground.at(x, z);
        let (d, half) = f.stream_distance(x, z);
        let wet = h <= WATER || d < half + Units::m(1.0);
        let path = !wet && f.on_path(x, z);
        let heathery = f.relief.patches.fx(x, z) > fx(0.47);
        let surface = if wet {
            K::bog
        } else if path {
            K::gravel
        } else if heathery {
            K::heather
        } else {
            K::grass
        };
        // Peat down to the world's bottom and no further, where hosts stop generating: the heath is solid, not a shell.
        let column = Column::new(h.0, spacing).surface(surface, m(0.5)).fill_below_exactly(K::peat, HEIGHTS.0);
        match surface {
            // Thickened only while it is at most four sites: taller, a medium stops rays skimming over the heath.
            K::heather if spacing <= 2 => column.medium(K::heather, m(0.4), 0.5),
            K::grass => column.medium(K::grass, 2, 0.25),
            _ => column,
        }
    })
}

/** The stream's bed, carved out of the ground: a channel a metre deep, at least a site wide. */
fn stream_bed(f: Rc<Moor>) -> Field {
    Field::at("stream bed", 1.0, Op::Carve, move |x, y, z, spacing| {
        let (d, half) = f.stream_distance(x, z);
        if d >= half.max(Units(spacing)) {
            return None;
        }
        let h = f.ground.at(x, z).0;
        // At least two sites deep: a line of sites along y has one every two.
        (y > h - m(1.0).max(2 * spacing) && y <= h + m(0.5)).then_some((0, None))
    })
}

/**
 * Water: in the stream's bed, a few units under its banks, and in the
 * hollows up to WATER. Painted, so at coarse levels, where the ground's
 * surface is two sites thick, its top site is water, not lost under it.
 */
fn water(f: Rc<Moor>) -> Field {
    Field::columns("water", 2.0, Op::Paint, move |x, z, spacing| {
        let h = f.ground.at(x, z).0;
        let (d, half) = f.stream_distance(x, z);
        if d < half.max(Units(spacing)) {
            return vec![Span::solid(h - m(1.0).max(2 * spacing), h - 1, K::water)];
        }
        Column::new(h, spacing).water_to(K::water, WATER.0).spans()
    })
}

/** Boulders half sunk in the heath, granite or lichened, off the water and the paths. */
fn boulders(f: Rc<Moor>) -> Population {
    let ground = f.clone();
    Population::new("boulders", 3.0, m(6.0)).reach(m(2.5)).anchor_on_ground(move |x, z| ground.ground.at(x, z).0).make(
        move |[x, y, z], c| {
            let mut random = c.random("boulder");
            let (xi, zi) = (x as i64, z as i64);
            if !random.chance(fx(0.1)) || y <= WATER.0 as f64 + 2.0 || f.stream_distance(xi, zi).0 < Units::m(4.0) || f.on_path(xi, zi) {
                return None;
            }
            let r = m(0.4) as f64 + random.next() * m(0.8) as f64;
            let radii = [r * (1.0 + random.next() * 0.6), r * 0.7, r * (1.0 + random.next() * 0.6)];
            let kind = if random.chance(fx(1.0 / 3.0)) { K::lichen } else { K::granite };
            Some(Node::of(vec![Shape::ellipsoid([x, y + r * 0.25, z], radii, kind)]))
        },
    )
}

/** The steadings: each owned by the cell its ring's centre is in. */
fn steadings_population(f: Rc<Moor>) -> Population {
    let anchor = f.clone();
    Population::new("steadings", 4.0, STEADING_CELL)
        .reach([REACH_ACROSS, m(16.0), REACH_ACROSS])
        .anchor(move |c| anchor.steading(c.centre[0], c.centre[2]).map(|s| [s.ring[0] as f64, s.ring_ground.0 as f64, s.ring[1] as f64]))
        .make(move |_, c| f.steading(c.centre[0], c.centre[2]).map(|s| steading(&f, &s)))
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
    part.add([-1, -m(1.5), -1], [length + 1, 0, depth + 1], K::granite);
    part.add([0, 0, 0], [length, top, depth], K::drystone);
    part.cut([WALL, 0, WALL], [length - WALL, top, depth - WALL]);
    let [(front, _), _, (back, back_length), _] = walls(length, depth);
    part.within(front).cut([length / 2 - 4, 0, 0], [length / 2 + 4, m(1.7), WALL]);
    part.within(back).cut([back_length / 2 - 2, 7, 0], [back_length / 2 + 2, 11, WALL]);
    // A lintel over the door, a hearth on the floor.
    part.within(front).add([length / 2 - 6, m(1.7), -1], [length / 2 + 6, m(1.7) + 2, 0], K::timber);
    part.ball([(length / 2) as f64, 0.5, (depth * 2 / 3) as f64], 2.5, K::granite, Op::Fill);
    // A thatched roof, one solid, overhanging all round: a skin of thatch over a stone gable
    // would be thinner than coarse sites, and they would show the stone.
    let rise = (depth as f64 / 2.0 + 3.0) * (0.9 + random.next() * 0.3);
    part.gable([-2.0, (top - 2) as f64, -3.0], [(length + 2) as f64, top as f64 + rise, (depth + 3) as f64], 0, K::thatch, Op::Fill);
    // From afar, measured: its walls and roof as they show, the room inside as the rays it lets through.
    // From afar, its outline. Measured (Far::measured), the roof's thatch is lost from above at levels 2 and 3
    // and what rays hit differs ~0.03 more at levels 2 and 3 in the contract.
    let mut far = Vec::new();
    let mut outline = Part::new(s.frame(), &mut far);
    outline.add([-1, -m(1.5), -1], [length + 1, top, depth + 1], K::drystone);
    outline.gable([-2.0, (top - 2) as f64, -3.0], [(length + 2) as f64, top as f64 + rise, (depth + 3) as f64], 0, K::thatch, Op::Fill);
    // Study: MOOR_HUT_MEASURED=1 measures it instead.
    if std::env::var("MOOR_HUT_MEASURED").is_ok() {
        return Node { far: Some(Far::measured(fixed(2.0))), ..Node::of(shapes) };
    }
    Node::of(shapes).with_far(Far::of(fixed(2.0), far))
}

/** A ring of standing stones round its centre, each a child; one of them fallen. */
fn ring(f: &Moor, s: &Steading, random: &mut Rng) -> Node {
    let mut children = Vec::new();
    let fallen = random.below(s.stones as u32) as i64;
    for i in 0..s.stones {
        let a = (i as f64 + random.next() * 0.3) * 2.0 * std::f64::consts::PI / s.stones as f64;
        let (x, z) = (s.ring[0] as f64 + s.radius as f64 * js::cos(a), s.ring[1] as f64 + s.radius as f64 * js::sin(a));
        let g = f.ground.at(x as i64, z as i64).0 as f64;
        let (tall, wide) = (m(1.6) as f64 + random.next() * m(1.6) as f64, 2.5 + random.next() * 1.5);
        let kind = if random.chance(Fx::HALF) { K::lichen } else { K::granite };
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
        let g = f.ground.at(x as i64, z as i64).0 as f64;
        let r = 2.5 + random.next();
        shapes.push(Shape::ball([x, g + r * 0.6, z], r, K::granite));
        shapes.push(Shape::ball([x, g + r * 1.6, z], r * 0.75, K::lichen));
        shapes.push(Shape::ball([x, g + r * 2.3, z], r * 0.5, K::granite));
        t += m(5.0) as f64;
        side = -side;
    }
    Node::of(shapes)
}
