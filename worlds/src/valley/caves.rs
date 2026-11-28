//! Caves in the mountains' faces, walked into. A mouth, framed by rocks,
//! opens where a steep slope rises into a hill; a passage winds in from
//! it, falling a little, to a chamber, and on from that to more, branching,
//! each passage and chamber under ground enough to keep it whole. The
//! chambers' roofs hang with dripstone over floors of rubble and
//! stalagmites, glow-worms light them, and deep in, one may hold a pool,
//! one crystals. Someone camped in the first chamber once.
//!
//! A part of the valley (valley.rs) that bids for no land: caves are
//! claimed anywhere (Claims), each laid out whole from its mouth and seed
//! (`Layout`), and dug by a field of shapes, carving the world (Field::shapes),
//! after everything else on the ground but the railway's bores, which they
//! keep off; what is in them stands in each chamber. The forest keeps off
//! their mouths (`clearing`), as a trunk over one would float.

use std::sync::Arc;

use ground::land::{clear_of, of_columns};
use ground::plan::Claims;
use worldgen::build::{dot, lerp, minus, plus, scale, unit};
use worldgen::curve::Curve;
use worldgen::generator::{ANY_SIZE, Child, Field, Kind, Layer, Node, Op, Point, Population, SOLID, Shape, ellipsoid, tube};
use worldgen::grid::Box3;
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, Scope, fx, hash64};
use worldgen::recent::Memo;
use worldgen::scalar::Scalar;
use worldgen::sites;
use worldgen::units::{Units, m};
use worldgen::world::Find;

use super::Terrain;
use super::mountains::SEA;
use super::railway::{self, Railway};
use super::town::K as T;
use crate::forest::K as F;

kinds! {
    base: railway::BASE + railway::KINDS.len() as u8;
    dripstone: 3, ["#e4d8bf", "#d8caac", "#ece2cc"];
    flowstone: 3, ["#c9a77c", "#bb9a70", "#d4b48a"];
    rubble: 3, ["#8c8476", "#7d7568", "#958d7f"];
    glowworm: 6, ["#b4fff2", "#96f5ea"], glows;
    crystal: 6, ["#d9a8ff", "#c48cff", "#e6c2ff"], glows;
    lantern: 7, ["#ffd98a", "#ffcf6e"], glows;
    ember: 3, ["#2b2622", "#3a332d"];
}

/**
 * Caves' mouths stand at least APART, claimed (Claims) from spots one in
 * each square CLAIM wide on average, a spot tried by CHANCE, at TRIES
 * places in it; CLAIM no coarser than the railway's line, which a claim
 * keeps off ("Bands" in docs/generators.md). Nothing of a cave is further
 * than REACH from its mouth along x or z.
 */
const CLAIM: i64 = m(256.0);
const APART: i64 = m(360.0);
const CHANCE: Fx = fx(0.25);
const TRIES: usize = 16;
const REACH: i64 = m(120.0);
/** How far from a mouth the forest keeps off, at the least. */
const CLEAR: i64 = m(12.0);
/** How far a chamber's things reach from its centre: its passage in, back to the chamber before it, or out of the mouth. */
const THING_REACH: [i64; 3] = [m(66.0), m(24.0), m(66.0)];
/** How far a passage's curve strays from its ends' line, each level: up and down a quarter as far (`bends`). */
const WANDER: [Fx; 3] = [Fx::m(2.4), Fx::m(1.2), Fx::m(0.6)];
/** The most it strays, all levels summed, along x or z, and up or down. */
const STRAY: f64 = m(4.2) as f64;
const STRAY_UP: f64 = STRAY / 4.0;
/**
 * The ground over a passage and over a chamber's roof, at the least; but
 * over a passage's first OPEN, out of the mouth and in under the face.
 */
const COVER: f64 = m(3.0) as f64;
const OPEN: f64 = m(17.0) as f64;
/** How deep under the mouth's ground its floor starts. */
const SILL: f64 = m(0.6) as f64;
/** What fills a chamber's bowl lies this far from its floor: rubble over it, water under it. */
const RUBBLE: f64 = m(0.3) as f64;
const SHORE: f64 = m(0.4) as f64;
/** The most chambers a cave has. */
const CHAMBERS: usize = 5;
/** How far round a chamber's doors its floor is clear of dripstone. */
const DOOR: f64 = m(3.0) as f64;

/** A cave where it is claimed: its mouth (x, the ground there, z), the way it goes in, and what it is laid out by. */
#[derive(Clone, Copy, Default, Debug)]
struct Cave {
    mouth: [i64; 3],
    heading: f64,
    seed: u64,
}

/**
 * A chamber: an ellipsoid, lobes off its walls, its floor where its
 * passage in comes; its bowl under that filled up to `level` with `fill`,
 * rubble to a little over its floor, or water a little under it: a pool.
 */
#[derive(Clone)]
struct Chamber {
    centre: Point,
    radii: Point,
    floor: f64,
    lobes: Vec<(Point, Point)>,
    fill: Kind,
    level: f64,
    /** Where its passages come in and go out, on its floor: dripstone keeps off them. */
    doors: Vec<Point>,
    crystals: bool,
    /** How many passages in from the mouth. */
    depth: usize,
}

/** A passage from one end to the other, its radius about `radius`, bending by `seed`. */
#[derive(Clone, Copy)]
struct Passage {
    from: Point,
    to: Point,
    radius: f64,
    seed: u64,
}

/** A cave laid out: passage i leads into chamber i, passage 0 in from the mouth. */
struct Layout {
    chambers: Vec<Chamber>,
    passages: Vec<Passage>,
    /** Its mouth, wider than the passage, dug into the face; and the rocks framing it, on the slope round it. */
    portal: Hollow,
    rocks: Vec<Rock>,
}

/** An ellipsoid: its centre and radii. */
type Hollow = (Point, Point);
/** A rock: an ellipsoid of a kind. */
type Rock = (Point, Point, Kind);

impl Chamber {
    /** How far (dx, dz) from its centre is out to its wall, squared: under one inside. */
    fn out(&self, dx: f64, dz: f64) -> f64 {
        (dx / self.radii[0]).powi(2) + (dz / self.radii[2]).powi(2)
    }

    /** Its roof (dx, dz) from its centre, if under it. */
    fn roof(&self, dx: f64, dz: f64) -> Option<f64> {
        let s = self.out(dx, dz);
        (s < 1.0).then(|| self.centre[1] + self.radii[1] * (1.0 - s).sqrt())
    }

    /** What lies under (dx, dz) from its centre, inside: its bowl, or what fills it. */
    fn floor_at(&self, dx: f64, dz: f64) -> f64 {
        let s = self.out(dx, dz).min(1.0);
        (self.centre[1] - self.radii[1] * (1.0 - s).sqrt()).max(self.level)
    }

    /** Whether (dx, dz) from its centre is clear of its doors. */
    fn clear(&self, dx: f64, dz: f64) -> bool {
        self.doors.iter().all(|d| (d[0] - self.centre[0] - dx).hypot(d[2] - self.centre[2] - dz) >= DOOR)
    }

    /** Its ellipsoid and lobes. */
    fn hollows(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        std::iter::once((self.centre, self.radii)).chain(self.lobes.iter().copied())
    }

    /** The box its hollows are in, `grown` each way. */
    fn bounds(&self, grown: f64) -> Box3 {
        bounds(self.hollows().flat_map(|(c, r)| [minus(c, r), plus(c, r)]), grown)
    }
}

impl Passage {
    /** Its centre line, as its tube is drawn along at level 0. */
    fn points(&self) -> Vec<Point> {
        bends(self.seed).points([self.from, self.to].map(|p| p.map(fx))).into_iter().map(|p| p.map(Fx::to_f64)).collect()
    }

    /** Its radius `t` of the way along: wider and narrower by turns. */
    fn radius(&self) -> impl Fn(f64) -> f64 + Send + Sync + 'static {
        let (r, phase) = (self.radius, (self.seed % 628) as f64 / 100.0);
        move |t: f64| r * (1.0 + 0.18 * js::sin(t * 9.0 + phase))
    }

    /** The box its tube is in, `grown` each way. */
    fn bounds(&self, grown: f64) -> Box3 {
        let stray = [STRAY, STRAY_UP, STRAY];
        let (lo, hi) = (min3(self.from, self.to), max3(self.from, self.to));
        bounds([minus(lo, stray), plus(hi, stray)], grown + 1.2 * self.radius)
    }
}

/**
 * A passage's bends, by `seed`: each point off its neighbours' middle by
 * a random offset that depends only on them, within its level's wander,
 * up or down a quarter as far as across, so its floor stays walkable.
 */
fn bends(seed: u64) -> Curve<3> {
    let radius: Vec<i64> = WANDER.iter().map(|w| w.to_bits()).collect();
    Curve::new(WANDER.to_vec(), move |level, a, b| {
        let key = a.iter().chain(&b).fold(hash64(seed, level as i64, 0, 0), |h, c| hash64(h, c.to_bits(), 0, 0));
        let (mut random, r) = (Rng::new(key), radius[level]);
        let off: [i64; 3] = loop {
            let mut off = [0; 3].map(|_| i64::from(random.below(2 * r as u32 + 1)) - r);
            off[1] /= 4;
            if off.iter().map(|o| o * o).sum::<i64>() <= r * r {
                break off;
            }
        };
        std::array::from_fn(|k| Fx::from_bits(((a[k].to_bits() + b[k].to_bits()) >> 1) + off[k]))
    })
}

/** Whether a hollow `radius` wide is dug at sites `spacing` apart: while wider than they are apart. */
fn dug(radius: f64, spacing: i64) -> bool {
    (spacing as f64) < 2.0 * radius
}

/** The ground over (x, z), lattice units. */
fn ground(terrain: &Terrain, x: f64, z: f64) -> f64 {
    terrain.ground(x as i64, z as i64).to_f64()
}

/** Whether the ground covers a hollow whose top is `top` at (x, z) by COVER or more, clear of the railway. */
fn covered(terrain: &Terrain, railway: &Railway, [x, _, z]: Point, top: f64) -> bool {
    ground(terrain, x, z) - top >= COVER && !railway.reaches(x as i64, z as i64, m(24.0))
}

/** `p` moved `d` along heading `a`, and `dy` up. */
fn ahead(p: Point, a: f64, d: f64, dy: f64) -> Point {
    [p[0] + d * js::cos(a), p[1] + dy, p[2] + d * js::sin(a)]
}

/**
 * The cave `cave` laid out on `terrain`, clear of `railway`, if it can be:
 * a passage in from 5 m out of its mouth to its first chamber, then from
 * each chamber passages on to more, by its seed, each the same rule
 * (`onward`), while they are covered, apart and near the mouth; none if
 * the first is not. A pool in one past the first, crystals in the deepest.
 */
fn layout(terrain: &Terrain, railway: &Railway, cave: &Cave) -> Option<Layout> {
    let mut random = Rng::new(cave.seed);
    let mouth = cave.mouth.map(|v| v as f64);
    let floor = mouth[1] - SILL;
    let start = ahead([mouth[0], floor, mouth[2]], cave.heading, -m(5.0) as f64, 0.0);
    let (entrance, first) = onward(terrain, railway, mouth, start, cave.heading, OPEN, 0, false, &mut random)?;
    let (portal, rocks) = mouth_of(terrain, cave, floor, &mut random);
    let mut layout = Layout { chambers: vec![first], passages: vec![entrance], portal, rocks };
    let mut pool = false;
    let mut from = 0;
    while from < layout.chambers.len() && layout.chambers.len() < CHAMBERS {
        let c = layout.chambers[from].clone();
        let exits = match c.depth {
            0 => 1 + random.below(2),
            1 => u32::from(random.chance(fx(0.7))),
            _ => u32::from(random.chance(fx(0.4))),
        };
        for _ in 0..exits {
            // Away from the mouth, give or take 70°, from its rim at its floor.
            let heading = js::atan2(c.centre[2] - mouth[2], c.centre[0] - mouth[0]) + (random.next() - 0.5) * 2.4;
            let rim = ahead([c.centre[0], c.floor, c.centre[2]], heading, 0.7 * c.radii[0].min(c.radii[2]), 0.0);
            let wants_pool = !pool && random.chance(Fx::HALF);
            let Some((passage, next)) = onward(terrain, railway, mouth, rim, heading, 0.0, c.depth + 1, wants_pool, &mut random) else {
                continue;
            };
            let apart = layout.chambers.iter().all(|o| {
                let d = ((o.centre[0] - next.centre[0]).powi(2) + (o.centre[2] - next.centre[2]).powi(2)).sqrt();
                d >= next.radii[0].max(next.radii[2]) + 0.8 * o.radii[0].max(o.radii[2])
            });
            if apart && layout.chambers.len() < CHAMBERS {
                pool |= wants_pool;
                layout.chambers[from].doors.push(rim);
                layout.chambers.push(next);
                layout.passages.push(passage);
            }
        }
        from += 1;
    }
    let deepest = (0..layout.chambers.len()).max_by_key(|&i| (layout.chambers[i].depth, usize::MAX - i)).expect("a chamber");
    layout.chambers[deepest].crystals = true;
    Some(layout)
}

/**
 * A passage from `from`, on its floor, along `heading`, falling a little,
 * and the chamber at its end, `depth` passages in from `mouth`, holding a
 * pool if it `pool`s: if the ground covers both past the passage's first
 * `open`, and the chamber is near enough the mouth.
 */
#[allow(clippy::too_many_arguments)]
fn onward(
    terrain: &Terrain,
    railway: &Railway,
    mouth: Point,
    from: Point,
    heading: f64,
    open: f64,
    depth: usize,
    pool: bool,
    random: &mut Rng,
) -> Option<(Passage, Chamber)> {
    let r = m(1.7) as f64 + random.next() * m(0.6) as f64;
    let length = open + m(16.0) as f64 + random.next() * m(16.0) as f64;
    let start = [from[0], from[1] + r, from[2]];
    let end = ahead(start, heading, length, -random.next() * 0.12 * (length - open));
    let passage = Passage { from: start, to: end, radius: r, seed: random.next_u64() };
    let steps = (length / m(4.0) as f64) as usize;
    let mut along = (0..=steps).map(|i| lerp(start, end, i as f64 / steps as f64)).filter(|p| distance2(*p, start).sqrt() >= open);
    if !along.all(|p| covered(terrain, railway, p, p[1] + r + STRAY_UP)) {
        return None;
    }
    let radii = [
        m(6.0) as f64 + random.next() * m(6.0) as f64,
        m(3.5) as f64 + random.next() * m(2.5) as f64,
        m(5.0) as f64 + random.next() * m(5.0) as f64,
    ];
    let floor = end[1] - r;
    let mut centre = ahead(end, heading, 0.8 * radii[0].min(radii[2]), 0.0);
    centre[1] = floor + 0.5 * radii[1];
    let lobes = (0..2 + random.below(3))
        .map(|_| {
            let (a, k) = (random.next() * std::f64::consts::TAU, 0.8 + random.next() * 0.3);
            let at = [centre[0] + 0.8 * radii[0] * js::cos(a), centre[1] - 0.2 * radii[1], centre[2] + 0.8 * radii[2] * js::sin(a)];
            (at, [0.5 * radii[0] * k, 0.6 * radii[1] * k, 0.5 * radii[2] * k])
        })
        .collect();
    let (fill, level): (Kind, f64) = if pool { (T::water.into(), floor - SHORE) } else { (K::rubble.into(), floor + RUBBLE) };
    let c = Chamber { centre, radii, floor, lobes, fill, level, doors: vec![[end[0], floor, end[2]]], crystals: false, depth };
    let near = c.bounds(0.0);
    let reach = [0, 2]
        .into_iter()
        .all(|a| (near.min[a] as f64 - mouth[a]).abs().max((near.max[a] as f64 - mouth[a]).abs()) <= (REACH - m(4.0)) as f64);
    let roofs = c.hollows().all(|(at, r)| {
        let top = at[1] + r[1];
        [(0.0, 0.0), (0.7, 0.0), (-0.7, 0.0), (0.0, 0.7), (0.0, -0.7)]
            .into_iter()
            .all(|(u, w)| covered(terrain, railway, [at[0] + u * r[0], 0.0, at[2] + w * r[2]], top))
    });
    (reach && roofs).then_some((passage, c))
}

/**
 * The mouth of `cave`, its floor at `floor`: a hollow wider than its
 * passage, just in from the face, and rocks round it on the slope, each
 * moved along the cave's heading to where the ground is at its height,
 * half sunk in it: a rocky arch, not a burrow.
 */
fn mouth_of(terrain: &Terrain, cave: &Cave, floor: f64, random: &mut Rng) -> (Hollow, Vec<Rock>) {
    let (a, mouth) = (cave.heading, cave.mouth.map(|v| v as f64));
    let (wide, tall) = (m(3.0) as f64 + random.next() * m(0.8) as f64, m(2.8) as f64 + random.next() * m(0.6) as f64);
    let centre = ahead([mouth[0], floor + tall - m(0.4) as f64, mouth[2]], a, m(1.5) as f64, 0.0);
    let side = [-js::sin(a), 0.0, js::cos(a)];
    let rocks = (0..9 + random.below(5))
        .filter_map(|_| {
            let (phi, ring) = ((random.next() * 2.0 - 1.0) * 1.9, wide + m(0.6) as f64 + random.next() * m(1.2) as f64);
            let at = plus(centre, [side[0] * ring * js::sin(phi), ring * js::cos(phi) * tall / wide, side[2] * ring * js::sin(phi)]);
            // Along the heading to where the ground is as high as it, half sunk.
            let d = (0..40).map(|k| k as f64 * m(0.5) as f64 - m(4.0) as f64).find(|&d| {
                let p = ahead(at, a, d, 0.0);
                ground(terrain, p[0], p[2]) >= at[1]
            })?;
            let r = m(0.9) as f64 + random.next() * m(1.1) as f64;
            let radii = [r * (0.8 + random.next() * 0.5), r * (0.6 + random.next() * 0.3), r * (0.8 + random.next() * 0.5)];
            let kind: Kind = if random.chance(fx(0.4)) { F::mossyStone.into() } else { F::stone.into() };
            Some((ahead(at, a, d - 0.4 * r, 0.0), radii, kind))
        })
        .collect();
    ((centre, [wide, tall, wide]), rocks)
}

/**
 * The cave claimed from `spot`, if any (by CHANCE): the first of a few
 * places round it on the mountains' faces where a steep slope rises into
 * a hill, well over the sea and away from the towns and the railway,
 * whose cave can be laid out. The landmarks, a finer band, it can't read:
 * COVER keeps their footings and ponds out of a cave.
 */
fn claim(terrain: &Terrain, railway: &Railway, spot: [i64; 2], random: &mut Rng) -> Option<Cave> {
    if !random.chance(CHANCE) {
        return None;
    }
    let g = |x: i64, z: i64| terrain.ground(x, z).to_f64();
    for _ in 0..TRIES {
        let (x, z) = (spot[0] - CLAIM / 2 + random.below(CLAIM as u32) as i64, spot[1] - CLAIM / 2 + random.below(CLAIM as u32) as i64);
        let across = terrain.course.across(x, z).abs();
        if across < Fx::m(200.0) || terrain.towns.distance(x, z) < Units::m(200.0) {
            continue;
        }
        let h = g(x, z);
        if h < (SEA.0 + m(6.0)) as f64 {
            continue;
        }
        let s = m(4.0);
        let (gx, gz) = ((g(x + s, z) - g(x - s, z)) / (2 * s) as f64, (g(x, z + s) - g(x, z - s)) / (2 * s) as f64);
        let slope = (gx * gx + gz * gz).sqrt();
        if !(0.7..=2.0).contains(&slope) {
            continue;
        }
        let heading = js::atan2(gz, gx);
        let into = |d: f64| {
            let p = ahead([x as f64, 0.0, z as f64], heading, d, 0.0);
            g(p[0] as i64, p[2] as i64)
        };
        // A face to open in: the ground rising steeply into a hill, falling away before it.
        let face = into(m(6.0) as f64) >= h + m(4.5) as f64 && into(-m(6.0) as f64) <= h - m(2.0) as f64;
        if !face || into(m(20.0) as f64) < h + m(10.0) as f64 || into(m(40.0) as f64) < h + m(16.0) as f64 || railway.reaches(x, z, m(40.0))
        {
            continue;
        }
        let cave = Cave { mouth: [x, terrain.ground(x, z).round_units().0, z], heading, seed: random.next_u64() };
        if layout(terrain, railway, &cave).is_some() {
            return Some(cave);
        }
    }
    None
}

/** The valley's caves, claimed anywhere on its mountains' faces. */
pub struct Caves {
    terrain: Arc<Terrain>,
    railway: Arc<Railway>,
    caves: Arc<Claims<Cave>>,
    layouts: Memo<[i64; 2], Option<Layout>>,
}

impl Caves {
    /** The caves on `terrain`, clear of `railway`. */
    pub fn new(scope: &Scope, terrain: Arc<Terrain>, railway: Arc<Railway>) -> Caves {
        let caves = {
            let (terrain, railway) = (terrain.clone(), railway.clone());
            Arc::new(Claims::new("caves", scope.seed("caves"), CLAIM, APART, move |spot, random| {
                claim(&terrain, &railway, spot, random).map(|c| ([c.mouth[0], c.mouth[2]], c))
            }))
        };
        Caves { terrain, railway, caves, layouts: Memo::new(64) }
    }

    /** The cave nearest the start of the path, within 64 km: its mouth. */
    pub fn first(&self) -> [i64; 3] {
        let start = self.terrain.path.x(0).round_units().0;
        self.caves.nearest(start, 0, m(64_000.0), |_| true).expect("a cave within 64 km of the start").1.mouth
    }

    /** The first chamber of the cave nearest the start of the path: a little over its floor. */
    pub fn first_chamber(&self) -> [i64; 3] {
        let mouth = self.first();
        let caves = self.within([mouth[0], mouth[2]], [mouth[0], mouth[2]]);
        let layout = caves.first().and_then(|(_, l)| l.as_ref().as_ref().map(|l| l.chambers[0].clone())).expect("the first cave's layout");
        let (x, z) = (layout.centre[0], layout.centre[2]);
        [x as i64, (layout.floor_at(0.0, 0.0) + m(1.7) as f64) as i64, z as i64]
    }

    /** The places it names: the first cave's mouth, and its first chamber. */
    pub fn places(self: &Arc<Self>) -> Vec<(&'static str, Find)> {
        let (mouth, chamber) = (self.clone(), self.clone());
        vec![("cave", Box::new(move || mouth.first())), ("grotto", Box::new(move || chamber.first_chamber()))]
    }

    /** The caves whose mouths are in columns `min` to `max`, with their layouts. */
    fn within(&self, min: [i64; 2], max: [i64; 2]) -> Vec<(Cave, Arc<Option<Layout>>)> {
        let caves = self.caves.within(min, max);
        caves.into_iter().map(|(at, cave)| (cave, self.layouts.of(&at, || layout(&self.terrain, &self.railway, &cave)))).collect()
    }

    /** The caves that may reach box `b`'s columns. */
    fn reaching(&self, b: &Box3) -> Vec<(Cave, Arc<Option<Layout>>)> {
        self.within([b.min[0] - REACH, b.min[2] - REACH], [b.max[0] - 1 + REACH, b.max[2] - 1 + REACH])
    }

    /** Where a cave may be: 1 within REACH of a mouth along x and z, else 0. */
    fn region(self: &Arc<Self>) -> Scalar {
        let (at, over) = (self.clone(), self.clone());
        let near = |caves: &Caves, min: [i64; 2], max: [i64; 2]| {
            !caves.caves.within([min[0] - REACH, min[1] - REACH], [max[0] + REACH, max[1] + REACH]).is_empty()
        };
        of_columns(
            move |x, z| if near(&at, [x, z], [x, z]) { Fx::ONE } else { Fx::ZERO },
            move |min, max| (Fx::ZERO, if near(&over, min, max) { Fx::ONE } else { Fx::ZERO }),
        )
    }

    /** How much of a column the forest keeps round the caves' mouths (`clear_of`): a trunk over one would float. */
    pub fn clearing(&self) -> Scalar {
        clear_of(self.caves.clone(), CLEAR)
    }

    /** The rocks round their mouths, their hollows, and what is in them. */
    pub fn layers(self: Arc<Self>) -> Vec<Layer> {
        layers(self)
    }
}

/** The rocks round the mouths of the caves reaching `b`. */
fn rocks(caves: &Caves, b: &Box3) -> Vec<Shape> {
    let mut shapes = Vec::new();
    for (_, layout) in caves.reaching(b) {
        let Some(layout) = layout.as_ref() else { continue };
        for &(at, r, kind) in &layout.rocks {
            if bounds([minus(at, r), plus(at, r)], 0.0).overlaps(b) {
                shapes.push(Shape::ellipsoid(at, r, kind));
            }
        }
    }
    shapes
}

/** The hollows of the caves reaching `b`, carved while wider than the sites are apart. */
fn hollows(caves: &Caves, b: &Box3) -> Vec<Shape> {
    let s = sites::spacing();
    let mut shapes = Vec::new();
    for (_, layout) in caves.reaching(b) {
        let Some(layout) = layout.as_ref() else { continue };
        for p in &layout.passages {
            if dug(p.radius, s) && p.bounds(0.0).overlaps(b) {
                shapes.push(tube(&bends(p.seed), p.from, p.to, p.radius(), 0, SOLID, Op::Carve));
            }
        }
        let mouth = std::iter::once(layout.portal);
        for (at, r) in layout.chambers.iter().flat_map(|c| c.hollows()).chain(mouth) {
            if dug(r[1], s) && bounds([minus(at, r), plus(at, r)], 0.0).overlaps(b) {
                shapes.push(ellipsoid(at, r, 0, SOLID, Op::Carve));
            }
        }
    }
    shapes
}

fn layers(caves: Arc<Caves>) -> Vec<Layer> {
    let region = caves.region();
    // Rocks round the mouths, on the slope: dug through where they are in a mouth.
    let rocks = {
        let caves = caves.clone();
        Field::shapes("caves' mouths", 7.9, Op::Fill, move |b| rocks(&caves, b)).within(region.clone(), 0)
    };
    // Dug after all that stands on the ground, so nothing over them reaches in; before the railway's bores, which they keep off.
    let dug = {
        let caves = caves.clone();
        Field::shapes("caves", 8.0, Op::Carve, move |b| hollows(&caves, b)).within(region.clone(), 0)
    };
    let things = {
        let anchor = caves.clone();
        Population::new("cave chambers", 8.5, m(16.0)).reach(THING_REACH).size(ANY_SIZE).within(region, 0).at(
            move |b| {
                let mut points = Vec::new();
                for (cave, layout) in anchor.reaching(b) {
                    let Some(l) = layout.as_ref() else { continue };
                    for (i, c) in l.chambers.iter().enumerate() {
                        points.push((c.centre, (layout.clone(), i, cave.seed)));
                    }
                }
                points
            },
            |(layout, i, seed), _| {
                let layout = layout.as_ref().as_ref().expect("a layout");
                Some(furnish(layout, *i, hash64(*seed, *i as i64, 0, 0)))
            },
        )
    };
    vec![Layer::Field(rocks), Layer::Field(dug), Layer::Population(things)]
}

// ---------------------------------------------------------------------------
// What is in a chamber: anchored at its centre, its passage in too.

/**
 * Chamber `i` of `layout`, furnished by `seed`: rubble on its floor,
 * dripstone hanging from its roof, stalagmites under, a column or two,
 * glow-worms on its roof and along its passage in; a pool or crystals if
 * it holds them; the camp in the first.
 */
fn furnish(layout: &Layout, i: usize, seed: u64) -> Node {
    let c = Arc::new(layout.chambers[i].clone());
    let passage = layout.passages[i];
    // Its dripstone reaches into its roof and floor.
    let within = c.bounds(m(1.0) as f64);
    let mut children = Vec::new();
    let lazy = |name: &str, bounds: Box3, n: i64, make: fn(&Chamber, &mut Rng) -> Vec<Shape>| {
        let c = c.clone();
        Child::new(name, ANY_SIZE, Node::lazy(bounds, hash64(seed, n, 0, 0), move |random| make(&c, random)))
    };
    // The stalagmites first: what fills the bowl fills round them.
    children.push(lazy("dripstone", within, 1, dripstone));
    children.push(lazy("bowl", within, 2, bowl));
    children.push(lazy("glow-worms", within, 3, glowworms));
    if c.crystals {
        children.push(lazy("crystals", within, 4, crystals));
    }
    if i == 0 {
        children.push(lazy("camp", within, 5, camp));
    }
    let along = passage.bounds(m(0.5) as f64);
    children.push(Child::new(
        "passage glow-worms",
        ANY_SIZE,
        Node::lazy(along, hash64(seed, 6, 0, 0), move |random| passage_glow(&passage, random)),
    ));
    Node::group(children)
}

/** What fills its bowl: all that is empty under its level, in its box. */
fn bowl(c: &Chamber, _: &mut Rng) -> Vec<Shape> {
    let b = c.bounds(0.0);
    vec![Shape::cuboid([b.min[0] as f64, b.min[1] as f64, b.min[2] as f64], [b.max[0] as f64, c.level, b.max[2] as f64], c.fill)]
}

/** A random point under its roof, `most` of the way out to its wall at the furthest (in the square of it). */
fn under_roof(c: &Chamber, random: &mut Rng, most: f64) -> (f64, f64) {
    loop {
        let (u, w) = ((random.next() * 2.0 - 1.0) * c.radii[0], (random.next() * 2.0 - 1.0) * c.radii[2]);
        if c.out(u, w) <= most {
            return (u, w);
        }
    }
}

/** Stalactites from its roof, stalagmites under some, columns where they met; its floor clear round its doors. */
fn dripstone(c: &Chamber, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let area = std::f64::consts::PI * c.radii[0] * c.radii[2];
    let n = (area / (m(2.2) * m(2.2)) as f64) as usize;
    for _ in 0..n {
        let (u, w) = under_roof(c, random, 0.7);
        let (x, z) = (c.centre[0] + u, c.centre[2] + w);
        let (roof, floor) = (c.roof(u, w).expect("under the roof"), c.floor_at(u, w));
        let room = roof - floor;
        let long = (m(0.3) as f64 + random.next().powi(2) * m(2.4) as f64).min(room - m(1.9) as f64);
        if long > 2.0 {
            let r = 1.0 + long * 0.12;
            shapes.push(Shape::lathe([x, roof + m(0.5) as f64, z], [x, roof - long, z], move |t: f64| r * (1.0 - 0.9 * t), K::dripstone));
        }
        if random.chance(fx(0.45)) && c.clear(u, w) {
            let tall = (m(0.3) as f64 + random.next() * m(1.5) as f64).min(room - long - m(1.9) as f64);
            if tall > 2.0 {
                let r = 2.0 + tall * 0.2;
                shapes.push(Shape::lathe(
                    [x, floor - m(0.5) as f64, z],
                    [x, floor + tall, z],
                    move |t: f64| r * (1.0 - 0.8 * t),
                    K::flowstone,
                ));
            }
        }
    }
    // A column or two, where a stalactite met its stalagmite: narrow in the middle, flaring to roof and floor.
    for _ in 0..1 + random.below(2) {
        let (u, w) = under_roof(c, random, 0.3);
        if !c.clear(u, w) {
            continue;
        }
        let (x, z) = (c.centre[0] + u, c.centre[2] + w);
        let (roof, floor) = (c.roof(u, w).expect("under the roof"), c.floor_at(u, w));
        let r = m(0.35) as f64 + random.next() * m(0.4) as f64;
        shapes.push(Shape::lathe(
            [x, floor - m(0.5) as f64, z],
            [x, roof + m(0.5) as f64, z],
            move |t: f64| r * (0.5 + 2.0 * (t - 0.5).powi(2)) * 1.6,
            K::flowstone,
        ));
    }
    shapes
}

/** Glow-worms in clusters on its roof: specks, each a little light. */
fn glowworms(c: &Chamber, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    for _ in 0..4 + random.below(4) {
        let (u0, w0) = under_roof(c, random, 0.6);
        for _ in 0..10 + random.below(10) {
            let (u, w) = (u0 + (random.next() - 0.5) * m(3.0) as f64, w0 + (random.next() - 0.5) * m(3.0) as f64);
            if let Some(roof) = c.roof(u, w) {
                let r = m(0.05) as f64 + random.next() * m(0.05) as f64;
                shapes.push(Shape::ball([c.centre[0] + u, roof, c.centre[2] + w], r, K::glowworm));
            }
        }
    }
    shapes
}

/** Glow-worms along a passage's walls and roof. */
fn passage_glow(p: &Passage, random: &mut Rng) -> Vec<Shape> {
    let (points, radius) = (p.points(), p.radius());
    let n = points.len() - 1;
    let mut shapes = Vec::new();
    for _ in 0..(n * 2) {
        let (j, t) = (random.below(n as u32) as usize, random.next());
        let (a, b) = (points[j], points[j + 1]);
        let d = unit(minus(b, a));
        // Across the passage, up more than down.
        let across = loop {
            let v = [random.next() * 2.0 - 1.0, random.next() * 2.0 - 1.0, random.next() * 2.0 - 1.0];
            let v = minus(v, scale(d, dot(v, d)));
            if dot(v, v) > 0.01 && v[1] / dot(v, v).sqrt() > -0.1 {
                break unit(v);
            }
        };
        let r = radius((j as f64 + t) / n as f64);
        shapes.push(Shape::ball(plus(lerp(a, b, t), scale(across, r)), m(0.05) as f64 + random.next() * m(0.05) as f64, K::glowworm));
    }
    shapes
}

/** Crystals growing out of its walls in clusters, pointing in. */
fn crystals(c: &Chamber, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    for _ in 0..8 + random.below(8) {
        let v = loop {
            let v = [random.next() * 2.0 - 1.0, random.next() * 1.4 - 0.4, random.next() * 2.0 - 1.0];
            if dot(v, v) > 0.04 && dot(v, v) <= 1.0 {
                break v;
            }
        };
        // Where the direction meets its wall, and in from there.
        let k = 1.0 / (0..3).map(|a| (v[a] / c.radii[a]).powi(2)).sum::<f64>().sqrt();
        let wall = plus(c.centre, scale(v, k));
        let normal = unit(minus(c.centre, wall));
        for _ in 0..3 + random.below(5) {
            let tilt = [0; 3].map(|_| (random.next() - 0.5) * 0.9);
            let along = unit(plus(normal, tilt));
            let long = m(0.5) as f64 + random.next() * m(1.2) as f64;
            let width = m(0.08) as f64 + random.next() * m(0.12) as f64;
            let (root, tip) = (minus(wall, scale(along, m(0.4) as f64)), plus(wall, scale(along, long)));
            shapes.push(Shape::capsule(root, tip, move |t: f64| width * ((1.0 - t) * 3.0).min(1.0), K::crystal));
        }
    }
    shapes
}

/** Someone's camp: a ring of stones round a dead fire, a crate, a lantern left on it, still lit. */
fn camp(c: &Chamber, random: &mut Rng) -> Vec<Shape> {
    let a = random.next() * std::f64::consts::TAU;
    let (u, w) = (0.35 * c.radii[0] * js::cos(a), 0.35 * c.radii[2] * js::sin(a));
    let (x, z, floor) = (c.centre[0] + u, c.centre[2] + w, c.floor_at(u, w));
    let mut shapes = Vec::new();
    for k in 0..8 {
        let a = k as f64 * std::f64::consts::TAU / 8.0;
        shapes.push(Shape::ball([x + m(0.5) as f64 * js::cos(a), floor, z + m(0.5) as f64 * js::sin(a)], m(0.16) as f64, F::stone));
    }
    shapes.push(Shape::ellipsoid([x, floor, z], [m(0.35) as f64, m(0.08) as f64, m(0.35) as f64], K::ember));
    for k in 0..3 {
        let a = a + k as f64 * 2.1;
        let (dx, dz) = (m(0.3) as f64 * js::cos(a), m(0.3) as f64 * js::sin(a));
        shapes.push(Shape::capsule([x - dx, floor + 1.0, z - dz], [x + dx, floor + 1.5, z + dz], 0.6, F::deadwood));
    }
    // The crate, a little off, and the lantern on it.
    let (cx, cz) = (x + m(1.4) as f64 * js::cos(a + 1.0), z + m(1.4) as f64 * js::sin(a + 1.0));
    let top = c.floor_at(cx - c.centre[0], cz - c.centre[2]) + m(0.5) as f64;
    shapes.push(Shape::cuboid(
        [cx - m(0.3) as f64, top - m(0.8) as f64, cz - m(0.25) as f64],
        [cx + m(0.3) as f64, top, cz + m(0.25) as f64],
        T::planks,
    ));
    shapes.push(Shape::lathe([cx, top, cz], [cx, top + m(0.25) as f64, cz], m(0.09) as f64, K::lantern));
    shapes.push(Shape::lathe([cx, top + m(0.25) as f64, cz], [cx, top + m(0.32) as f64, cz], m(0.06) as f64, T::iron));
    shapes
}

// ---------------------------------------------------------------------------
// Points' bounds.

fn distance2(a: Point, b: Point) -> f64 {
    let d = minus(a, b);
    dot(d, d)
}

fn min3(a: Point, b: Point) -> Point {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]
}

fn max3(a: Point, b: Point) -> Point {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

/** The box of whole units holding `points`, `grown` each way. */
fn bounds(points: impl IntoIterator<Item = Point>, grown: f64) -> Box3 {
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in points {
        lo = min3(lo, p);
        hi = max3(hi, p);
    }
    Box3 { min: lo.map(|v| (v - grown).floor() as i64 - 1), max: hi.map(|v| (v + grown).ceil() as i64 + 2) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::valley::valley;
    use lattice::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};
    use worldgen::generator::generate;

    /** What is in each chamber keeps within its population's reach, of caves as they come: drawing checks only the few it makes. */
    #[test]
    fn chambers_keep_within_their_reach() {
        let world = Scope::new(3);
        let terrain = Arc::new(Terrain::new(&world));
        let railway = Arc::new(Railway::new(&world.of("railway"), terrain.clone()));
        let caves = Caves::new(&world.of("caves"), terrain, railway);
        let found = caves.within([-m(6000.0); 2], [m(6000.0); 2]);
        assert!(found.len() > 20, "{} caves", found.len());
        for (cave, layout) in found {
            let layout = layout.as_ref().as_ref().expect("a layout");
            for (i, c) in layout.chambers.iter().enumerate() {
                let anchor = c.centre.map(|v| v.floor() as i64);
                let within =
                    Box3 { min: [0, 1, 2].map(|a| anchor[a] - THING_REACH[a]), max: [0, 1, 2].map(|a| anchor[a] + THING_REACH[a] + 1) };
                let bounds = furnish(layout, i, cave.seed).bounds();
                assert!(bounds.inside(&within), "chamber {i} of the cave at {:?} reaches {bounds:?}", cave.mouth);
            }
        }
    }

    /** A body walks in from outside a cave's mouth along its passage, and stands at its end, in its first chamber. */
    #[test]
    fn a_body_walks_into_a_cave() {
        for seed in [1, 42] {
            let world = Scope::new(seed);
            let terrain = Arc::new(Terrain::new(&world));
            let railway = Arc::new(Railway::new(&world.of("railway"), terrain.clone()));
            let caves = Caves::new(&world.of("caves"), terrain, railway);
            let mouth = caves.first();
            let found = caves.within([mouth[0], mouth[2]], [mouth[0], mouth[2]]);
            let layout = found[0].1.as_ref().as_ref().expect("a layout");
            let (way, chamber) = (layout.passages[0].points(), &layout.chambers[0]);
            let near = bounds(way.iter().copied(), m(6.0) as f64);
            let level = COLLISION_LEVEL;
            let area = Box3 { min: near.min.map(|v| v >> level), max: near.max.map(|v| (v >> level) + 1) };
            let tiles = lattice::Tiles::one(generate(&valley(seed).generator, &area, level).to_grid());
            let ground = Ground { tiles: &tiles };
            let mut body = Walker::at([way[0][0], way[0][1] + m(2.0) as f64, way[0][2]]);
            let mut next = 1;
            for _ in 0..60 * 60 {
                let ([x, _, z], to) = (body.eye(), way[next]);
                if (to[0] - x).hypot(to[2] - z) < m(1.0) as f64 {
                    if next + 1 == way.len() {
                        break;
                    }
                    next += 1;
                }
                let yaw = (to[0] - x).atan2(to[2] - z).to_degrees();
                body = body.walk(WalkInput { forward: 1.0, yaw, ..WalkInput::default() }, 1.0 / 60.0, &ground);
            }
            // At its end, on the rubble its passage comes in on.
            let ([x, y, z], end) = (body.eye(), way[way.len() - 1]);
            let (at_end, standing) =
                ((end[0] - x).hypot(end[2] - z) < m(1.0) as f64, (m(1.4) as f64..m(2.1) as f64).contains(&(y - chamber.level)));
            assert!(
                at_end && standing,
                "seed {seed}: eye at {:?}, the passage's end at {end:?}, its floor at {}",
                body.eye(),
                chamber.level
            );
        }
    }
}
