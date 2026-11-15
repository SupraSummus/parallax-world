//! The cheese: endless every way, holed through with caverns a few metres
//! wide, tunnels between them and eyes in their walls, lit by magic
//! crystals growing from the caverns' walls. No way is up but for a body
//! walking it (bake/src/walk.rs): it climbs the caverns' floors, walks the
//! tunnels, and a pillar's winding stairs in each cavern lead up into the
//! one over it. The caverns overlap enough to make one maze.
//! Written from the engine's public API alone (docs/generators.md).
//!
//! Caverns and eyes are balls, one in each cell of a cubic grid of their
//! own; tunnels capsules from low in a cavern to low in a neighbour's.
//! A field carves them line by line of sites along y, each a chord along
//! it, so at any level as many sites as the hole holds, on average; only
//! those that reach the stretch of the line asked for.

use worldgen::World;
use worldgen::generator::{Child, Field, Generator, Layer, Meets, Node, Op, Point, Population, Shape, Span};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Rng, hash64, stream};
use worldgen::plan::spacing;
use worldgen::units::m;

kinds! {
    base: 0;
    cheese: 2, ["#f3d46c", "#efcd5f", "#f6da7a"];
    amethyst: 6, ["#b884ff", "#a36cf7"], glows;
    aquamarine: 6, ["#74e6ff", "#5cd3f2"], glows;
    rose: 6, ["#ff86dc", "#f572cf"], glows;
}

/** Balls of one size: one in a cell CELL wide with the chance `chance`, its radius from `least` to `most`. */
struct Balls {
    cell: i64,
    least: i64,
    most: i64,
    chance: f64,
}

/**
 * Caverns: as many as overlap into one maze (a ball of their mean volume
 * per cell is 0.48 of it, so 38 % of the cheese is caverns, over the 29 %
 * at which balls' union reaches through all of it).
 */
const CAVERNS: Balls = Balls { cell: m(16.0), least: m(5.0), most: m(10.0), chance: 1.0 };
/** Eyes: smaller, in the walls of the caverns and on their own. */
const EYES: Balls = Balls { cell: m(6.0), least: m(1.0), most: m(2.5), chance: 0.6 };
const _: () = assert!(CAVERNS.most <= CAVERNS.cell && EYES.most <= EYES.cell, "a ball reaches into the cells beside its own only");

/**
 * Tunnels from a cavern to its neighbours along x and z, level or a
 * layer of cells down, each with its chance; wide enough to walk, their
 * radius from TUNNEL_LEAST to TUNNEL_MOST.
 */
const TUNNELS: [([i64; 3], f64); 4] = [([1, 0, 0], 0.5), ([0, 0, 1], 0.5), ([1, -1, 0], 0.35), ([0, -1, 1], 0.35)];
const TUNNEL_LEAST: i64 = m(1.2);
const TUNNEL_MOST: i64 = m(1.8);

/**
 * Crystals stand in the cheese at most NEAR behind a cavern's wall, in
 * clusters about SPREAD apart, each reaching LENGTH out of it at most.
 * What stands in a cavern is made while sites are SIZE apart at most
 * (level 3): a host's cube a level coarser holds eight times the caverns.
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
        if rng.next() >= self.chance {
            return None;
        }
        let centre = [i, j, k].map(|c| c * self.cell + (rng.next() * self.cell as f64) as i64);
        let radius = self.least + (rng.next() * (self.most - self.least + 1) as f64) as i64;
        Some((centre, radius))
    }

    /** The balls that may reach the box [min, max]. */
    fn balls(&self, seed: u64, min: [i64; 3], max: [i64; 3]) -> impl Iterator<Item = ([i64; 3], i64)> + '_ {
        let [lo, hi] = [min.map(|v| v - self.most), max.map(|v| v + self.most)].map(|p| p.map(|v| v.div_euclid(self.cell)));
        cells(lo, hi).filter_map(move |c| self.ball(seed, c))
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

/** A tunnel: a capsule from `from` to `to`. */
struct Tunnel {
    from: Point,
    to: Point,
    radius: f64,
}

/** Where a tunnel starts in a cavern: half way down from its centre. */
fn low((centre, radius): ([i64; 3], i64)) -> Point {
    [centre[0] as f64, (centre[1] - radius / 2) as f64, centre[2] as f64]
}

/** The tunnels that may reach the box [min, max]. */
fn tunnels(seeds: Seeds, min: [i64; 3], max: [i64; 3]) -> impl Iterator<Item = Tunnel> {
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
            if rng.next() >= chance {
                return None;
            }
            let from = low(CAVERNS.ball(seeds.caverns, a)?);
            let to = low(CAVERNS.ball(seeds.caverns, [0, 1, 2].map(|i| a[i] + d[i]))?);
            let radius = (TUNNEL_LEAST + (rng.next() * (TUNNEL_MOST - TUNNEL_LEAST + 1) as f64) as i64) as f64;
            let reaches = (0..3).all(|i| from[i].min(to[i]) - radius <= max[i] as f64 && from[i].max(to[i]) + radius >= min[i] as f64);
            reaches.then_some(Tunnel { from, to, radius })
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

impl Tunnel {
    fn holds(&self, p: Point) -> bool {
        let (d, w) = (minus(self.to, self.from), minus(p, self.from));
        let t = (dot(w, d) / dot(d, d)).clamp(0.0, 1.0);
        let off = minus(w, d.map(|v| v * t));
        dot(off, off) < self.radius * self.radius
    }

    /**
     * Where the line of sites along y at (x, z) is inside it: (lo, hi).
     * It is convex, so one stretch: from the lowest to the highest of the
     * balls round its ends' and the cylinder's between them.
     */
    fn chord(&self, x: f64, z: f64) -> Option<(f64, f64)> {
        let r2 = self.radius * self.radius;
        let mut chord = (f64::INFINITY, f64::NEG_INFINITY);
        let mut add = |lo: f64, hi: f64| chord = (chord.0.min(lo), chord.1.max(hi));
        for end in [self.from, self.to] {
            let across = (x - end[0]).powi(2) + (z - end[2]).powi(2);
            if across < r2 {
                let half = (r2 - across).sqrt();
                add(end[1] - half, end[1] + half);
            }
        }
        // The cylinder: the point at height from + s is off its axis by a quadratic in s, along it by e + s·dy.
        let d = minus(self.to, self.from);
        let (dd, level) = (dot(d, d), d[0] * d[0] + d[2] * d[2]);
        let (ax, az) = (x - self.from[0], z - self.from[2]);
        let e = ax * d[0] + az * d[2];
        let (a, b, c) = (level / dd, -2.0 * e * d[1] / dd, ax * ax + az * az - e * e / dd - r2);
        let disc = b * b - 4.0 * a * c;
        if level > 0.0 && disc > 0.0 {
            let (mut lo, mut hi) = ((-b - disc.sqrt()) / (2.0 * a), (-b + disc.sqrt()) / (2.0 * a));
            // Between its ends: 0 ≤ e + s·dy ≤ dd.
            if d[1] != 0.0 {
                let (s0, s1) = (-e / d[1], (dd - e) / d[1]);
                (lo, hi) = (lo.max(s0.min(s1)), hi.min(s0.max(s1)));
            } else if !(0.0..=dd).contains(&e) {
                (lo, hi) = (0.0, -1.0);
            }
            if lo < hi {
                add(self.from[1] + lo, self.from[1] + hi);
            }
        }
        (chord.0 < chord.1).then_some(chord)
    }
}

/** The holes that may reach a box: read once, for a thing that asks of many points in it. */
struct Holes {
    caverns: Vec<([i64; 3], i64)>,
    eyes: Vec<([i64; 3], i64)>,
    tunnels: Vec<Tunnel>,
}

impl Holes {
    fn near(seeds: Seeds, min: [i64; 3], max: [i64; 3]) -> Holes {
        Holes {
            caverns: CAVERNS.balls(seeds.caverns, min, max).collect(),
            eyes: EYES.balls(seeds.eyes, min, max).collect(),
            tunnels: tunnels(seeds, min, max).collect(),
        }
    }

    /** Whether `p` is in one. */
    fn open(&self, p: Point) -> bool {
        self.caverns.iter().chain(&self.eyes).any(|ball| outside(p, *ball) < 0.0) || self.tunnels.iter().any(|t| t.holds(p))
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

/** Whether holes of the widest `most` are carved at sites `spacing` apart: while wider than the sites are apart. */
fn carved(most: i64, spacing: i64) -> bool {
    spacing < 2 * most
}

/**
 * The holes: each one's chord along each line of sites, carved; each kind
 * while its widest is wider than the sites are apart, finer detail left
 * out ("Fields" in "Levels", docs/generators.md): a host asks for cubes,
 * so a tile coarser by a level holds eight times the holes. Nothing
 * where none of those carved reaches, a site or two around it.
 */
fn holes(seeds: Seeds) -> Field {
    Field::spans("holes", 1.0, Op::Carve, 1, move |x, z, stretch| {
        let ((from, to), s) = (stretch, spacing());
        let (low, high) = ([x, from, z], [x, to, z]);
        let (fx, fz) = (x as f64, z as f64);
        let carved = |most: i64| carved(most, s);
        let caverns = carved(CAVERNS.most).then(|| CAVERNS.balls(seeds.caverns, low, high));
        let eyes = carved(EYES.most).then(|| EYES.balls(seeds.eyes, low, high));
        let tunnels = carved(TUNNEL_MOST).then(|| tunnels(seeds, low, high));
        let chords = caverns
            .into_iter()
            .flatten()
            .chain(eyes.into_iter().flatten())
            .filter_map(|([cx, cy, cz], r)| {
                let (across, r2) = (((x - cx).pow(2) + (z - cz).pow(2)) as f64, (r * r) as f64);
                (across < r2).then(|| (cy as f64 - (r2 - across).sqrt(), cy as f64 + (r2 - across).sqrt()))
            })
            .chain(tunnels.into_iter().flatten().filter_map(|t| t.chord(fx, fz)));
        chords
            .filter_map(|(lo, hi)| {
                let (from, to) = (lo.ceil() as i64, hi.floor() as i64 + 1);
                (from < to).then(|| Span::solid(from, to, K::cheese))
            })
            .collect::<Vec<_>>()
    })
    .meets(move |b, s| {
        let [min, max] = [b.min, b.max.map(|v| v - 1)];
        let reaches = |(c, r): ([i64; 3], i64)| (0..3).all(|i| c[i] - r <= max[i] && c[i] + r >= min[i]);
        let balls = |balls: &Balls, seed: u64| carved(balls.most, s) && balls.balls(seed, min, max).any(reaches);
        let tunnels = carved(TUNNEL_MOST, s) && tunnels(seeds, min, max).next().is_some();
        if balls(&CAVERNS, seeds.caverns) || balls(&EYES, seeds.eyes) || tunnels { Meets::Any } else { Meets::Nothing }
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

/** The cell of N·FCC holding `p`: its centre's nearest site, i + j + k even (on a face, either). */
fn fcc_cell(p: [i64; 3], n: i64) -> [i64; 3] {
    let f = p.map(|v| v as f64 / n as f64);
    let mut c = f.map(f64::round);
    if c.iter().sum::<f64>().rem_euclid(2.0) != 0.0 {
        // Of the even sites, the nearest moves the coordinate furthest from its rounding the other way.
        let i = (0..3).max_by(|&a, &b| (f[a] - c[a]).abs().total_cmp(&(f[b] - c[b]).abs())).expect("three");
        c[i] += if f[i] > c[i] { 1.0 } else { -1.0 };
    }
    c.map(|v| v as i64)
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
        .anchor(move |cell| {
            // The caverns' cells it meets: it reaches n from its centre, a cavern's cell is 2n wide.
            let [lo, hi] = [-n, n - 1].map(|d| cell.centre.map(|v| (v + d).div_euclid(CAVERNS.cell)));
            let ijk = [cell.i, cell.j, cell.k];
            let (centre, _) = cells(lo, hi).filter_map(|c| CAVERNS.ball(seeds.caverns, c)).find(|(c, _)| fcc_cell(*c, n) == ijk)?;
            Some(centre.map(|v| v as f64))
        })
        .make(move |p, cell| {
            let at = p.map(|v| v as i64);
            let (centre, radius) = CAVERNS.ball(seeds.caverns, at.map(|v| v.div_euclid(CAVERNS.cell)))?;
            // All it asks of: its wall, a little behind it, and over its ceiling as high as its stairs may climb.
            let reach = radius + CLIMB + m(3.0);
            let holes = Holes::near(seeds, at.map(|v| v - reach), at.map(|v| v + reach));
            let mut rng = cell.random("clusters");
            let tries = (4.0 * std::f64::consts::PI * (radius * radius) as f64 / (SPREAD * SPREAD) as f64) as usize;
            let clusters = (0..tries).filter_map(|n| {
                let out = direction(&mut rng);
                let behind = radius as f64 + 1.0 + rng.next() * (NEAR - 1) as f64;
                let p = [0, 1, 2].map(|i| centre[i] as f64 + out[i] * behind);
                let mut rng = Rng::new(rng.next_u64());
                (holes.wall_near(p) == Some(centre)).then(|| Child::new(format!("cluster {n}"), SIZE, cluster(p, centre, &mut rng)))
            });
            let stairs = Child::new("stairs", SIZE, stairs(&holes, (centre, radius), &mut cell.random("stairs")));
            Some(Node::group(std::iter::once(stairs).chain(clusters).collect()))
        })
}

/** The cheese of a seed; the camera starts beside the pillar of the cavern of the origin's cell. */
pub fn cheese(seed: u64) -> World {
    let seeds = Seeds { caverns: stream(seed, 1), eyes: stream(seed, 2), tunnels: stream(seed, 3) };
    let ([x, y, z], _) = CAVERNS.ball(seeds.caverns, [0, 0, 0]).expect("a cavern in every cell");
    // Clear of the stairs, on a site: x + y + z even.
    let x = x + m(3.5);
    let spawn = [x, y - (x + y + z).rem_euclid(2), z];
    let layers = vec![Layer::Field(body()), Layer::Field(holes(seeds)), Layer::Population(grottoes(seeds))];
    World { generator: Generator { kinds: KINDS.to_vec(), seed: stream(seed, 4), layers }, spawn, heights: None, points: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** A tunnel's chord along a line of sites holds the points of the line the tunnel holds, and only them. */
    #[test]
    fn a_tunnels_chord_is_where_it_holds_a_line() {
        let mut rng = Rng::new(7);
        for _ in 0..200 {
            let mut point = || [0; 3].map(|_| (rng.next() - 0.5) * 256.0);
            let tunnel = Tunnel { from: point(), to: point(), radius: 4.0 + rng.next() * 12.0 };
            let [x, _, z] = [0, 1, 2].map(|i| tunnel.from[i] + (rng.next() - 0.5) * 48.0);
            let chord = tunnel.chord(x, z);
            for y in (-200..200).map(|y| y as f64 + 0.5) {
                let inside = chord.is_some_and(|(lo, hi)| (lo..=hi).contains(&y));
                assert_eq!(inside, tunnel.holds([x, y, z]), "at ({x}, {y}, {z}): chord {chord:?}");
            }
        }
    }
}
