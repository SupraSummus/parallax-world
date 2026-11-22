//! The cheese: endless every way, holed through with caverns a few metres
//! wide, tunnels between them and eyes in their walls, lit by magic
//! crystals growing from the caverns' walls. No way is up but for a body
//! walking it (fcc/src/walk.rs): it climbs the caverns' floors, walks the
//! tunnels, and a pillar's winding stairs in each cavern lead up into the
//! one over it. The caverns overlap enough to make one maze.
//! Written from the engine's public API alone (docs/generators.md).
//!
//! Caverns and eyes are balls, one in each cell of a cubic grid of their
//! own; tunnels capsules from low in a cavern to low in a neighbour's.
//! A field of them carves the world (Field::shapes): those that may reach
//! the box asked for. Crystals too small for a level's sites are there as
//! what they are on average (`far`): along the caverns' walls, glowing
//! pieces of sites, as dense as the crystals measured, so they light from afar.

use std::sync::{Arc, OnceLock};

use worldgen::World;
use worldgen::entry::{Entry, Known};
use worldgen::generator::{Child, Field, Fill, Generator, Layer, Meets, Node, Op, Point, Population, Shadows, Shape, ball};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, fx, hash64, stream};
use worldgen::sites::spacing;
use worldgen::units::m;

kinds! {
    base: 0;
    cheese: 2, ["#f3d46c", "#efcd5f", "#f6da7a"];
    amethyst: 6, ["#b884ff", "#a36cf7"], glows;
    aquamarine: 6, ["#74e6ff", "#5cd3f2"], glows;
    rose: 6, ["#ff86dc", "#f572cf"], glows;
    // The crystals far off: their shades.
    crystals: 6, ["#b884ff", "#a36cf7", "#74e6ff", "#5cd3f2", "#ff86dc", "#f572cf"], glows;
}

/** Balls of one size: one in a cell CELL wide with the chance `chance`, its radius from `least` to `most`. */
struct Balls {
    cell: i64,
    least: i64,
    most: i64,
    chance: Fx,
}

/**
 * Caverns: as many as overlap into one maze (a ball of their mean volume
 * per cell is 0.48 of it, so 38 % of the cheese is caverns, over the 29 %
 * at which balls' union reaches through all of it).
 */
const CAVERNS: Balls = Balls { cell: m(16.0), least: m(5.0), most: m(10.0), chance: Fx::ONE };
/** Eyes: smaller, in the walls of the caverns and on their own. */
const EYES: Balls = Balls { cell: m(6.0), least: m(1.0), most: m(2.5), chance: fx(0.6) };
const _: () = assert!(CAVERNS.most <= CAVERNS.cell && EYES.most <= EYES.cell, "a ball reaches into the cells beside its own only");

/**
 * Tunnels from a cavern to its neighbours along x and z, level or a
 * layer of cells down, each with its chance; wide enough to walk, their
 * radius from TUNNEL_LEAST to TUNNEL_MOST.
 */
const TUNNELS: [([i64; 3], Fx); 4] = [([1, 0, 0], Fx::HALF), ([0, 0, 1], Fx::HALF), ([1, -1, 0], fx(0.35)), ([0, -1, 1], fx(0.35))];
const TUNNEL_LEAST: i64 = m(1.2);
const TUNNEL_MOST: i64 = m(1.8);

/**
 * Crystals stand in the cheese at most NEAR behind a cavern's wall, in
 * clusters about SPREAD apart, each reaching LENGTH out of it at most.
 * What stands in a cavern is made while sites are SIZE apart at most
 * (level 3): a host's cube a level coarser holds eight times the caverns.
 * Further apart, the crystals are their mean (`far`).
 */
const NEAR: i64 = m(0.75);
const SPREAD: i64 = m(6.0);
const LENGTH: i64 = m(3.5);
const SIZE: i64 = m(1.0);
/**
 * A cavern's pillar and its stairs: square steps RISE apart, a sixth of
 * a turn, wide enough to overlap, a turn over a body's height (walk.rs:
 * it steps up 0.55 m, and is 1.8 m tall). They climb while there is room
 * over them, up through an open ceiling CLIMB past the cavern's at most.
 */
const PILLAR: f64 = m(1.0) as f64;
const STEP: f64 = m(0.95) as f64;
const RISE: i64 = m(0.5);
const THICK: f64 = m(0.3) as f64;
const HEADROOM: f64 = m(2.2) as f64;
const CLIMB: i64 = m(6.0);

impl Balls {
    /** The ball of cell (i, j, k), its centre and radius, if it has one. */
    fn ball(&self, seed: u64, [i, j, k]: [i64; 3]) -> Option<([i64; 3], i64)> {
        let mut rng = Rng::new(hash64(seed, i, j, k));
        if !rng.chance(self.chance) {
            return None;
        }
        let centre = [i, j, k].map(|c| c * self.cell + rng.below(self.cell as u32) as i64);
        let radius = self.least + rng.below((self.most - self.least + 1) as u32) as i64;
        Some((centre, radius))
    }

    /** The balls that may reach the box [min, max]. */
    fn balls(&self, seed: u64, min: [i64; 3], max: [i64; 3]) -> impl Iterator<Item = ([i64; 3], i64)> + '_ {
        let [lo, hi] = [min.map(|v| v - self.most), max.map(|v| v + self.most)].map(|p| p.map(|v| v.div_euclid(self.cell)));
        cells(lo, hi).filter_map(move |c| self.ball(seed, c))
    }

    /** Those carved at sites `spacing` apart (`carved`): none, if none of them is so wide. */
    fn carved(&self, seed: u64, min: [i64; 3], max: [i64; 3], spacing: i64) -> impl Iterator<Item = ([i64; 3], i64)> + '_ {
        let wide = carved(self.most, spacing).then(|| self.balls(seed, min, max).filter(move |&(_, r)| carved(r, spacing)));
        wide.into_iter().flatten()
    }
}

/** The cells from `lo` to `hi`, both included. */
fn cells(lo: [i64; 3], hi: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    (lo[0]..=hi[0]).flat_map(move |i| (lo[1]..=hi[1]).flat_map(move |j| (lo[2]..=hi[2]).map(move |k| [i, j, k])))
}

/** The world's seeds: the caverns', the eyes', the tunnels'. */
#[derive(Clone, Copy)]
struct Seeds {
    caverns: u64,
    eyes: u64,
    tunnels: u64,
}

impl Seeds {
    fn of(seed: u64) -> Seeds {
        Seeds { caverns: stream(seed, 1), eyes: stream(seed, 2), tunnels: stream(seed, 3) }
    }
}

/** Where a tunnel starts in a cavern: half way down from its centre. */
fn low((centre, radius): ([i64; 3], i64)) -> Point {
    [centre[0] as f64, (centre[1] - radius / 2) as f64, centre[2] as f64]
}

/** The tunnels that may reach the box [min, max], capsules carving, those wider than `spacing`. */
fn tunnels(seeds: Seeds, min: [i64; 3], max: [i64; 3], spacing: i64) -> impl Iterator<Item = Shape> {
    TUNNELS.iter().enumerate().flat_map(move |(n, &(d, chance))| {
        // A tunnel lies in its caverns' cells, a radius past them, and along y half a cavern's under them (`low`).
        let [lo, hi] = [0, 1].map(|end| {
            [0, 1, 2].map(|a| {
                let pad = TUNNEL_MOST + if a == 1 { CAVERNS.most / 2 } else { 0 };
                let v = if end == 0 { min[a] - pad } else { max[a] + pad };
                v.div_euclid(CAVERNS.cell) - if end == 0 { d[a].max(0) } else { d[a].min(0) }
            })
        });
        cells(lo, hi).filter_map(move |a| {
            let mut rng = Rng::new(hash64(seeds.tunnels ^ n as u64, a[0], a[1], a[2]));
            if !rng.chance(chance) {
                return None;
            }
            let from = low(CAVERNS.ball(seeds.caverns, a)?);
            let to = low(CAVERNS.ball(seeds.caverns, [0, 1, 2].map(|i| a[i] + d[i]))?);
            let radius = TUNNEL_LEAST + rng.below((TUNNEL_MOST - TUNNEL_LEAST + 1) as u32) as i64;
            carved(radius, spacing).then(|| Shape::capsule(from, to, radius as f64, K::cheese).carve())
        })
    })
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/** `v` made a unit long. */
fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = dot(v, v).sqrt();
    v.map(|c| c / l)
}

/** How far `p` is outside the ball at `centre` of `radius`: under 0 inside it. */
fn outside(p: Point, (centre, radius): ([i64; 3], i64)) -> f64 {
    let d = minus(p, centre.map(|v| v as f64));
    dot(d, d).sqrt() - radius as f64
}

/** The holes that may reach a box: read once, for a thing that asks of many points in it. */
struct Holes {
    caverns: Vec<([i64; 3], i64)>,
    /** All of them, caverns too. */
    shapes: Vec<Shape>,
}

impl Holes {
    fn near(seeds: Seeds, min: [i64; 3], max: [i64; 3]) -> Holes {
        let caverns: Vec<_> = CAVERNS.balls(seeds.caverns, min, max).collect();
        let eyes = EYES.balls(seeds.eyes, min, max);
        let shapes = caverns.iter().copied().chain(eyes).map(hole).chain(tunnels(seeds, min, max, 0)).collect();
        Holes { caverns, shapes }
    }

    /** Whether `p` is in one. */
    fn open(&self, p: Point) -> bool {
        self.shapes.iter().any(|shape| shape.holds(p))
    }

    /** If `p` is in the cheese, the centre of the cavern whose wall is nearest, if within NEAR. */
    fn wall_near(&self, p: Point) -> Option<[i64; 3]> {
        let (d, (centre, _)) = self.caverns.iter().map(|ball| (outside(p, *ball), *ball)).min_by(|a, b| a.0.total_cmp(&b.0))?;
        ((0.0..=NEAR as f64).contains(&d) && !self.open(p)).then_some(centre)
    }
}

/** The cheese, everywhere: all of any box. */
fn body() -> Field {
    Field::at("cheese", 0.0, Op::Fill, |_, _, _| Some((K::cheese.into(), None))).meets(|_, _| Meets::All(K::cheese.into()))
}

/** Whether a hole of `radius` is carved at sites `spacing` apart: while wider than the sites are apart. */
fn carved(radius: i64, spacing: i64) -> bool {
    spacing < 2 * radius
}

/** A ball's hole. */
fn hole((centre, radius): ([i64; 3], i64)) -> Shape {
    Shape::ball(centre.map(|v| v as f64), radius as f64, K::cheese).carve()
}

/**
 * The holes: each carved while wider than the sites are apart, finer
 * detail left out ("Fields" in "Levels", docs/generators.md): a host
 * asks for cubes, so a tile coarser by a level holds eight times the
 * holes, and a kind none of which is that wide is not asked for.
 */
fn holes(seeds: Seeds) -> Field {
    Field::shapes("holes", 1.0, Op::Carve, move |b| {
        let ([min, max], s) = ([b.min, b.max.map(|v| v - 1)], spacing());
        let tunnels = carved(TUNNEL_MOST, s).then(|| tunnels(seeds, min, max, s)).into_iter().flatten();
        CAVERNS.carved(seeds.caverns, min, max, s).chain(EYES.carved(seeds.eyes, min, max, s)).map(hole).chain(tunnels)
    })
}

/** A cluster of crystals at `p`, in the cheese, pointing into the cavern centred at `into`. */
fn cluster(p: Point, into: [i64; 3], rng: &mut Rng) -> Node {
    let kind = [K::amethyst, K::aquamarine, K::rose][rng.below(3) as usize];
    let normal = unit(minus(into.map(|v| v as f64), p));
    let shapes = (0..4 + rng.below(6))
        .map(|_| {
            let tilt = [0; 3].map(|_| (rng.next() - 0.5) * 0.9);
            let along = unit([0, 1, 2].map(|i| normal[i] + tilt[i]));
            // Through the cheese to the wall, then out into the cavern.
            let length = NEAR as f64 + (0.35 + 0.65 * rng.next()) * LENGTH as f64;
            let tip = [0, 1, 2].map(|i| p[i] + along[i] * length);
            let width = m(0.15) as f64 + rng.next() * m(0.25) as f64;
            // Narrowing to a point over its last third.
            Shape::capsule(p, tip, move |t: f64| width * ((1.0 - t) * 3.0).min(1.0), kind)
        })
        .collect();
    Node::of(shapes)
}

/**
 * The clusters of crystals on the wall of the cavern `ball`, by `rng`:
 * each its number, where it is in the cheese, and what it is made by.
 */
fn clusters<'a>(holes: &'a Holes, (centre, radius): ([i64; 3], i64), mut rng: Rng) -> impl Iterator<Item = (usize, Point, Rng)> + 'a {
    let tries = (4.0 * std::f64::consts::PI * (radius * radius) as f64 / (SPREAD * SPREAD) as f64) as usize;
    (0..tries).filter_map(move |n| {
        let out = direction(&mut rng);
        let behind = radius as f64 + behind(&mut rng);
        let p = [0, 1, 2].map(|i| centre[i] as f64 + out[i] * behind);
        let rng = Rng::new(rng.next_u64());
        (holes.wall_near(p) == Some(centre)).then_some((n, p, rng))
    })
}

/** How far behind its cavern's wall a cluster is. */
fn behind(rng: &mut Rng) -> f64 {
    1.0 + rng.next() * (NEAR - 1) as f64
}

/**
 * The crystals measured, the same in every cheese: what each slab of a
 * cluster adds to its shadow over directions, a cluster on a wall that
 * faces up (lattice units, y from the wall into the cavern); and how
 * many clusters a cavern's wall holds a unit², of caverns near the origin.
 */
struct Crystals {
    shadows: Shadows,
    per_area: f64,
    /** By the spacing's power of two, what each slab a spacing thick adds, from the first a site may read (`slab`). */
    slabs: [OnceLock<Vec<f64>>; 64],
}

impl Crystals {
    /** What the slab from `from`, `s` thick, adds to a cluster's shadow (Shadows::slab): measured once a spacing. */
    fn slab(&self, from: i64, s: i64) -> f64 {
        let first = -(NEAR + s + 1);
        let slabs = self.slabs[s.trailing_zeros() as usize]
            .get_or_init(|| (first..NEAR + LENGTH).map(|from| self.shadows.slab(from, from + s)).collect());
        slabs.get((from - first) as usize).copied().unwrap_or(0.0)
    }
}

/** Clusters the crystals are measured over (Crystals); their caverns, those of 4³ cells about the origin. */
const MEASURED: usize = 64;

fn crystals() -> &'static Crystals {
    static CRYSTALS: OnceLock<Crystals> = OnceLock::new();
    CRYSTALS.get_or_init(|| {
        let (seeds, measuring) = (Seeds::of(0), stream(0, 5));
        let mut rng = Rng::new(measuring);
        let sample: Vec<Node> = (0..MEASURED)
            .map(|_| {
                let p = [0.0, -behind(&mut rng), 0.0];
                let mut rng = Rng::new(rng.next_u64());
                cluster(p, [0, CAVERNS.most, 0], &mut rng)
            })
            .collect();
        let (mut found, mut area) = (0usize, 0.0);
        for c in cells([-2; 3], [1; 3]) {
            let Some((centre, radius)) = CAVERNS.ball(seeds.caverns, c) else { continue };
            let reach = radius + m(1.0);
            let holes = Holes::near(seeds, centre.map(|v| v - reach), centre.map(|v| v + reach));
            found += clusters(&holes, (centre, radius), Rng::new(hash64(measuring, c[0], c[1], c[2]))).count();
            area += 4.0 * std::f64::consts::PI * (radius * radius) as f64;
        }
        Crystals { shadows: Shadows::new(&sample, 0, 1), per_area: found as f64 / area, slabs: std::array::from_fn(|_| OnceLock::new()) }
    })
}

/**
 * The crystals at sites further apart than SIZE: about each cavern the
 * holes carve at the spacing, a ball of them strewn along its wall, as
 * dense as they are (Shadows: extinction = clusters a unit³ times their
 * mean shadow) at each site's depth, a spacing thick; denser inwards, as
 * the wall's sphere shrinks.
 */
fn far(seeds: Seeds) -> Field {
    Field::shapes("crystals", 2.0, Op::Fill, move |b| {
        let ([min, max], s) = ([b.min, b.max.map(|v| v - 1)], spacing());
        CAVERNS.carved(seeds.caverns, min.map(|v| v - NEAR), max.map(|v| v + NEAR), s).map(|(centre, radius)| {
            let at = centre.map(|v| v as f64);
            let fill = move |x: f64, y: f64, z: f64| {
                let (s, depth) = (spacing(), -outside([x, y, z], (centre, radius)));
                let (crystals, half) = (crystals(), s as f64 / 2.0);
                let shrunk = (radius as f64 / (radius as f64 - depth).max(1.0)).powi(2);
                let extinction = crystals.per_area * shrunk * crystals.slab((depth - half).floor() as i64, s) / s as f64;
                Fill::stopping(extinction)
            };
            ball(at, (radius + NEAR) as f64, K::crystals.into(), Fill::At(Arc::new(fill)), Op::Fill)
        })
    })
}

/** A random direction, every one as likely. */
fn direction(rng: &mut Rng) -> [f64; 3] {
    loop {
        let v = [0; 3].map(|_| rng.next() * 2.0 - 1.0);
        if (0.01..=1.0).contains(&dot(v, v)) {
            return unit(v);
        }
    }
}

/** A cavern's pillar, floor to ceiling, and its stairs winding up from its floor while there is room over each step. */
fn stairs(holes: &Holes, (centre, radius): ([i64; 3], i64), rng: &mut Rng) -> Node {
    let [x, y, z] = centre.map(|v| v as f64);
    let (floor, ceiling) = (y - radius as f64, y + radius as f64);
    let mut shapes = vec![Shape::lathe([x, floor - m(1.0) as f64, z], [x, ceiling + m(1.0) as f64, z], PILLAR, K::cheese)];
    let (turn, out) = (rng.next() * std::f64::consts::TAU, PILLAR + 0.6 * STEP);
    for n in 1.. {
        let top = floor + (n * RISE) as f64;
        let angle = turn + n as f64 * std::f64::consts::TAU / 6.0;
        let [sx, sz] = [x + out * js::cos(angle), z + out * js::sin(angle)];
        if top > ceiling + CLIMB as f64 || !holes.open([sx, top + 1.0, sz]) || !holes.open([sx, top + HEADROOM, sz]) {
            break;
        }
        shapes.push(Shape::cuboid([sx - STEP, top - THICK, sz - STEP], [sx + STEP, top, sz + STEP], K::cheese));
    }
    Node::of(shapes)
}

/**
 * What stands in the caverns: in each, the thing of the cell its centre
 * is in, its pillar and stairs, and clusters of crystals scattered on its
 * wall, each kept where it is in the cheese and this wall is the nearest.
 */
fn grottoes(seeds: Seeds) -> Population {
    let n = CAVERNS.cell / 2;
    Population::new("grottoes", 2.0, n)
        .reach(CAVERNS.most + CLIMB + m(1.0))
        .size(SIZE)
        .far(far(seeds))
        .anchor(move |cell| {
            // The caverns' cells it meets: it reaches n from its centre, a cavern's cell is 2n wide.
            let [lo, hi] = [-n, n - 1].map(|d| cell.centre.map(|v| (v + d).div_euclid(CAVERNS.cell)));
            // The cavern whose centre is in it: the cell owns its thing.
            cells(lo, hi).filter_map(|c| CAVERNS.ball(seeds.caverns, c)).map(|(c, _)| c.map(|v| v as f64)).find(|&c| cell.owns(c))
        })
        .make(move |p, cell| {
            let at = p.map(|v| v as i64);
            let (centre, radius) = CAVERNS.ball(seeds.caverns, at.map(|v| v.div_euclid(CAVERNS.cell)))?;
            // All it asks of: its wall, a little behind it, and over its ceiling as high as its stairs may climb.
            let reach = radius + CLIMB + m(3.0);
            let holes = Holes::near(seeds, at.map(|v| v - reach), at.map(|v| v + reach));
            let clusters = clusters(&holes, (centre, radius), cell.random("clusters"))
                .map(|(n, p, mut rng)| Child::new(format!("cluster {n}"), SIZE, cluster(p, centre, &mut rng)));
            let stairs = Child::new("stairs", SIZE, stairs(&holes, (centre, radius), &mut cell.random("stairs")));
            Some(Node::group(std::iter::once(stairs).chain(clusters).collect()))
        })
}

/**
 * The cheese, checked to level 7, past which the caverns are buried
 * (`carved`). From level 4 on, caves have no pillars or stairs, and their
 * crystals are their far form (SIZE).
 */
pub const CHEESE: Entry = Entry {
    name: "cheese",
    make: cheese,
    levels: 7,
    accuracy: &[],
    // Why: docs/todo.md, "Five of the cheese's tiles are over the cost budget".
    known: &[
        Known::OverBudget("spawn", 3),
        Known::OverBudget("spawn", 4),
        Known::OverBudget("spawn", 5),
        Known::OverBudget("spawn", 6),
        Known::OverBudget("spawn", 7),
    ],
};

/** The worlds of this crate (worldgen's entry.rs). */
pub const WORLDS: &[Entry] = &[CHEESE];

/** The cheese of a seed; the camera starts beside the pillar of the cavern of the origin's cell. */
pub fn cheese(seed: u64) -> World {
    let seeds = Seeds::of(seed);
    let ([x, y, z], _) = CAVERNS.ball(seeds.caverns, [0, 0, 0]).expect("a cavern in every cell");
    // Clear of the stairs, on a site: x + y + z even.
    let x = x + m(3.5);
    let spawn = [x, y - (x + y + z).rem_euclid(2), z];
    let layers = vec![Layer::Field(body()), Layer::Field(holes(seeds)), Layer::Population(grottoes(seeds))];
    World { generator: Generator { kinds: KINDS.to_vec(), seed: stream(seed, 4), layers }, spawn, heights: None, points: Vec::new() }
}
