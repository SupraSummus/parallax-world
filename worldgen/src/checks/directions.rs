//! Whether a world's directions follow the lattice's: the rotational
//! sibling of `places` (docs/generators.md, "Checks"). About the vertical
//! the lattice, square noise and tiles are four-fold, so their mark is a
//! four-fold harmonic of horizontal directions at the lattice's phase,
//! the same at every seed.
//!
//! The measure: pairs of points a few metres apart, at random places and
//! in continuous random directions φ, and whether the two differ in kind
//! (nothing being a kind too). Over the pairs that do, the mean of cos 4φ
//! is the harmonic: positive if pairs part most along the axes, negative
//! along the diagonals (as across walls along the axes). Points are
//! anywhere on a plane of sites, not on the sites, so that rounding to
//! them leaves no mark; and both of a pair are on one plane: the next
//! plane's sites are on the other columns, and a ground flatter than a
//! site over a pair's length would round to the two in a checkerboard.
//!
//! A world's own direction makes a four-fold harmonic too: pairs part
//! more the more across it they look, and no such pattern has a four-fold
//! harmonic larger than its two-fold one, while pairs are shorter than
//! the gaps between the rows (walls 3 m thick, pairs 16 m: nearly as
//! large). So only the excess counts, and a world with a strong direction
//! of its own hides as much of the lattice's.
//!
//! Pairs near each other are alike, so seeds are the samples: the
//! excess's spread over seeds, each left out in turn (a jackknife), gives
//! Student's t.

use std::f64::consts::{PI, SQRT_2, TAU};
use std::fmt;

use crate::World;
use crate::generator::{Generator, generate};
use crate::grid::{BRICK, Box3};
use crate::noise::{Rng, hash_text};
use crate::picture::nearest_site;

/** The level pairs are sampled at. */
const LEVEL: u32 = 3;
/** How far apart pairs are, sites of LEVEL: well over one, and short of the gaps between a world's rows. */
const DISTANCES: [i64; 3] = [4, 8, 16];
/** Boxes are this many sites a side in x and z, over the heights. */
const SIDE: i64 = 128;
/** Boxes are around the spawn, each place the world names, and this many anywhere, at each seed. */
const ANYWHERE: usize = 4;
/** How wide a square around spawn boxes anywhere are in, lattice units (4 km). */
const AROUND: i64 = 32_000;
/** Points sampled in each box at each distance, each with DIRECTIONS pairs. */
const POINTS: usize = 32_768;
/** Directions evenly spread over a half turn, turned together by a random angle at each point. */
const DIRECTIONS: usize = 8;
/** The chance of a false alarm per world, over all the tests. */
pub const ALPHA: f64 = 1e-3;

/** What pairs at a distance add up to, over some boxes. */
#[derive(Clone, Copy, Default, Debug)]
struct Sums {
    pairs: f64,
    /** Pairs whose points differ in kind. */
    parted: f64,
    /** Over those: cos 4φ; cos 2φ and sin 2φ. */
    four: f64,
    two: [f64; 2],
}

impl Sums {
    fn add(&mut self, other: &Sums) {
        self.pairs += other.pairs;
        self.parted += other.parted;
        self.four += other.four;
        self.two = [self.two[0] + other.two[0], self.two[1] + other.two[1]];
    }

    /** The four-fold harmonic at the lattice's phase, and the two-fold one, relative to how often pairs part. */
    fn harmonics(&self) -> (f64, [f64; 2]) {
        let n = self.parted.max(1.0);
        (2.0 * self.four / n, [2.0 * self.two[0] / n, 2.0 * self.two[1] / n])
    }

    /** The four-fold harmonic beyond what the two-fold one may make of it, along the axes (`sign` 1) or the diagonals (−1). */
    fn excess(&self, sign: f64) -> f64 {
        let (four, [c, s]) = self.harmonics();
        sign * four - c.hypot(s)
    }
}

/** Each material id's kind, plus one; 0 for none. */
fn kinds(generator: &Generator) -> Vec<u16> {
    let mut kinds = vec![0];
    for (i, kind) in generator.kinds.iter().enumerate() {
        kinds.extend(kind.colors.iter().map(|_| i as u16 + 1));
    }
    kinds
}

/** Pairs at each of DISTANCES in a box around `at` (x and z, lattice units, at a random place in the box), over the world's heights. */
fn sample(world: &World, kinds: &[u16], at: [i64; 3], random: &mut Rng, sums: &mut [Sums; DISTANCES.len()]) {
    let (y0, y1) = world.heights_at(LEVEL);
    let [x0, z0] = [at[0], at[2]].map(|v| (v >> LEVEL) - random.below(SIDE as u32) as i64);
    let grid = generate(&world.generator, &Box3 { min: [x0, y0, z0], max: [x0 + SIDE, y1, z0 + SIDE] }, LEVEL);
    // Over the highest brick with voxels, pairs never part.
    let [bx, by] = [0, 1].map(|i| (grid.n[i] + BRICK - 1) / BRICK);
    let highest = grid.bricks.iter().enumerate().filter(|(_, at)| **at >= 0).map(|(i, _)| (i as i64 / bx) % by).max();
    let Some(top) = highest.map(|j| (grid.min[1] + (j + 1) * BRICK).min(y1)) else { return };
    let kind = |p: [f64; 3]| {
        let [x, y, z] = nearest_site(p);
        kinds[grid.get(x, y, z) as usize]
    };
    let spread: [[f64; 2]; DIRECTIONS] = std::array::from_fn(|k| {
        let a = PI * k as f64 / DIRECTIONS as f64;
        [a.cos(), a.sin()]
    });
    for (sums, &d) in sums.iter_mut().zip(&DISTANCES) {
        let (d, inside) = (d as f64, (SIDE - 2 * d - 2) as f64);
        for _ in 0..POINTS {
            // On a plane of sites, a whole y, so both of a pair round to its sites (see above).
            let (x, y) = (x0 as f64 + d + 1.0 + random.next() * inside, (y0 + random.below((top - y0) as u32) as i64) as f64);
            let p = [x, y, z0 as f64 + d + 1.0 + random.next() * inside];
            let (here, turn) = (kind(p), random.next() * TAU);
            let (c0, s0) = (turn.cos(), turn.sin());
            for [ck, sk] in spread {
                // cos φ and sin φ, φ the turn and the k-th direction; then of 2φ and 4φ.
                let (c, s) = (c0 * ck - s0 * sk, s0 * ck + c0 * sk);
                sums.pairs += 1.0;
                if kind([p[0] + d * c, p[1], p[2] + d * s]) != here {
                    let (c2, s2) = (c * c - s * s, 2.0 * s * c);
                    sums.parted += 1.0;
                    sums.four += c2 * c2 - s2 * s2;
                    sums.two = [sums.two[0] + c2, sums.two[1] + s2];
                }
            }
        }
    }
}

/**
 * Pairs at each of DISTANCES in `world` around each of its places: a box
 * around its spawn, around each place it names, and ANYWHERE in a square
 * AROUND its spawn wide (together, as "anywhere"), each box picked by the
 * world's seed.
 */
fn sums(world: &World) -> Vec<(&'static str, [Sums; DISTANCES.len()])> {
    let mut random = Rng::new(hash_text("directions", world.generator.seed));
    let anywhere: Vec<[i64; 3]> = (0..ANYWHERE)
        .map(|_| {
            let mut off = || random.below(AROUND as u32) as i64 - AROUND / 2;
            [world.spawn[0] + off(), world.spawn[1], world.spawn[2] + off()]
        })
        .collect();
    let named = std::iter::once(("spawn", vec![world.spawn])).chain(world.points.iter().map(|(name, find)| (*name, vec![find()])));
    let kinds = kinds(&world.generator);
    named
        .chain(std::iter::once(("anywhere", anywhere)))
        .map(|(name, boxes)| {
            let mut sums = [Sums::default(); DISTANCES.len()];
            for at in boxes {
                sample(world, &kinds, at, &mut random, &mut sums);
            }
            (name, sums)
        })
        .collect()
}

/** A world's four-fold harmonic beyond its two-fold one, around a place, at the distance and the phase where it is least likely by chance. */
#[derive(Clone, Debug)]
pub struct Leak {
    pub place: &'static str,
    /** Tests made: around each place, at each distance, at the axes' phase and the diagonals'. */
    pub tests: usize,
    /** How far apart pairs are, sites of LEVEL. */
    pub distance: i64,
    /** The four-fold harmonic at the lattice's phase, relative to how often pairs part: most along the axes if positive, the diagonals if negative. */
    pub four: f64,
    /** The two-fold harmonic's size, and the direction pairs part most along, radians from x towards z. */
    pub two: f64,
    pub along: f64,
    /** Seeds, and the share of pairs that part. */
    pub seeds: usize,
    pub parted: f64,
    /** The chance an excess this large shows by chance in one of the tests (Bonferroni). */
    pub chance: f64,
}

impl Leak {
    /** Whether the world's directions follow the lattice's: a chance below ALPHA. */
    pub fn broken(&self) -> bool {
        self.chance < ALPHA
    }
}

impl fmt::Display for Leak {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let metres = (self.distance << LEVEL) as f64 * crate::units::UNIT_SIZE;
        let axes = if self.four >= 0.0 { "axes" } else { "diagonals" };
        write!(
            f,
            "{} seeds, {} tests; around {}, pairs {metres} m apart ({:.1} % of them part): \
             a four-fold harmonic of {:+.2} % (most along the lattice's {axes}) against a two-fold one of {:.2} % \
             (most along {:.0}° from x), by chance {:.1e}",
            self.seeds,
            self.tests,
            self.place,
            100.0 * self.parted,
            100.0 * self.four,
            100.0 * self.two,
            self.along.to_degrees(),
            self.chance
        )?;
        if self.broken() {
            write!(f, ": its directions follow the lattice's (see \"Checks\" in docs/generators.md)")?;
        }
        Ok(())
    }
}

/** The chance Student's t with `nu` degrees of freedom is over `t`: Wallace's normal approximation, a few percent over it. */
fn t_tail(t: f64, nu: f64) -> f64 {
    let z = (8.0 * nu + 1.0) / (8.0 * nu + 3.0) * (nu * (t * t / nu).ln_1p()).sqrt();
    libm::erfc(t.signum() * z / SQRT_2) / 2.0
}

/**
 * The four-fold harmonic beyond the two-fold one around `place`, at the
 * distance and phase least likely by chance, `sums` holding each seed's
 * pairs there; its chance over `tests` tests.
 */
fn leak(place: &'static str, sums: &[&[Sums; DISTANCES.len()]], tests: usize) -> Leak {
    let k = sums.len() as f64;
    let total = |d: usize, but: Option<usize>| {
        let mut all = Sums::default();
        sums.iter().enumerate().filter(|&(s, _)| Some(s) != but).for_each(|(_, s)| all.add(&s[d]));
        all
    };
    // The largest t, and its distance.
    let mut best = (f64::NEG_INFINITY, 0);
    for d in 0..DISTANCES.len() {
        for sign in [1.0, -1.0] {
            // Each seed left out in turn: the jackknife's spread.
            let left: Vec<f64> = (0..sums.len()).map(|out| total(d, Some(out)).excess(sign)).collect();
            let mean = left.iter().sum::<f64>() / k;
            let spread = ((k - 1.0) / k * left.iter().map(|e| (e - mean) * (e - mean)).sum::<f64>()).sqrt();
            let t = total(d, None).excess(sign) / spread.max(1e-12);
            if t > best.0 {
                best = (t, d);
            }
        }
    }
    let (t, d) = best;
    let all = total(d, None);
    let (four, two) = all.harmonics();
    Leak {
        place,
        tests,
        distance: DISTANCES[d],
        four,
        two: two[0].hypot(two[1]),
        along: two[1].atan2(two[0]) / 2.0,
        seeds: sums.len(),
        parted: all.parted / all.pairs.max(1.0),
        chance: (t_tail(t, k - 1.0) * tests as f64).min(1.0),
    }
}

/**
 * Whether the worlds `make` makes at `seeds` (a thread each, three at
 * least) follow the lattice's directions: around each of their places,
 * the strongest four-fold harmonic beyond the two-fold one, with its
 * chance over all the tests; the strongest first.
 */
pub fn directions(make: impl Fn(u64) -> World + Sync, seeds: &[u64]) -> Vec<Leak> {
    assert!(seeds.len() >= 3, "seeds are the samples: three at least");
    let make = &make;
    let sums: Vec<_> = std::thread::scope(|scope| {
        let threads: Vec<_> = seeds.iter().map(|&seed| scope.spawn(move || sums(&make(seed)))).collect();
        threads.into_iter().map(|t| t.join().expect("sampled")).collect()
    });
    let places = sums[0].len();
    let tests = places * 2 * DISTANCES.len();
    let mut leaks: Vec<Leak> = (0..places).map(|p| leak(sums[0][p].0, &sums.iter().map(|s| &s[p].1).collect::<Vec<_>>(), tests)).collect();
    leaks.sort_by(|a, b| a.chance.total_cmp(&b.chance));
    leaks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{Field, KindDef, Layer, Op, Span};

    const SEEDS: &[u64] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    const GROUND: u8 = 0;
    const WALL: u8 = 1;

    /** The strongest four-fold harmonic beyond the two-fold one of the worlds `make` makes, around any place. */
    fn strongest(make: impl Fn(u64) -> World + Sync, seeds: &[u64]) -> Leak {
        directions(make, seeds).remove(0)
    }

    /** A world of ground up to `ground`, and walls 6 m high where `wall` says, at (x, z). */
    fn world(seed: u64, ground: impl Fn(f64, f64) -> f64 + 'static, wall: impl Fn(f64, f64) -> bool + 'static) -> World {
        let kinds = ["ground", "wall"].map(|name| KindDef {
            name,
            kind_type: 1,
            colors: &["#808080"],
            walk_through: false,
            glows: false,
            medium: false,
        });
        let field = Field::columns("land", 0.0, Op::Fill, move |x, z, _| {
            let (x, z) = (x as f64, z as f64);
            let top = ground(x, z).round() as i64;
            let mut spans = vec![Span { from: -64, to: top, kind: GROUND, extinction: None }];
            if wall(x, z) {
                spans.push(Span { from: top, to: top + 48, kind: WALL, extinction: None });
            }
            spans
        });
        World {
            generator: Generator { kinds: kinds.to_vec(), seed, layers: vec![Layer::Field(field)] },
            spawn: [0; 3],
            heights: (-64, 128),
            points: Vec::new(),
        }
    }

    /** Hills of four dozen waves 10 to 75 m long, in directions anywhere. */
    fn hills(seed: u64) -> impl Fn(f64, f64) -> f64 {
        let mut random = Rng::new(seed);
        let waves: Vec<[f64; 4]> =
            (0..48).map(|_| [random.next() * TAU, 80.0 + random.next() * 520.0, random.next() * TAU, 4.0 + random.next() * 8.0]).collect();
        move |x, z| waves.iter().map(|[a, l, p, h]| h * (TAU * (x * a.cos() + z * a.sin()) / l + p).cos()).sum()
    }

    /** Walls 3 m thick 24 m apart, across x only or both ways, turned by `turn` radians. */
    fn walls(both: bool, turn: f64) -> impl Fn(f64, f64) -> bool {
        move |x, z| {
            let (u, v) = (x * turn.cos() + z * turn.sin(), z * turn.cos() - x * turn.sin());
            u.rem_euclid(192.0) < 24.0 || (both && v.rem_euclid(192.0) < 24.0)
        }
    }

    #[test]
    fn hills_and_walls_turned_any_way_follow_no_lattice() {
        let bare = strongest(|seed| world(seed, hills(seed), |_, _| false), SEEDS);
        assert!(!bare.broken(), "hills: {bare}");
        // So gentle that pairs a few metres apart are mostly less than a site apart in height.
        let gentle = strongest(
            |seed| {
                let hills = hills(seed);
                world(seed, move |x, z| hills(x, z) / 8.0, |_, _| false)
            },
            SEEDS,
        );
        assert!(!gentle.broken(), "gentle hills: {gentle}");
        // Walls a seed turns its own way.
        let turned = strongest(|seed| world(seed, |_, _| 0.0, walls(true, Rng::new(seed).next() * TAU)), SEEDS);
        assert!(!turned.broken(), "turned: {turned}");
        // Walls along z, the world's own direction.
        let rows = strongest(|seed| world(seed, |_, _| 0.0, walls(false, 0.0)), SEEDS);
        assert!(!rows.broken(), "rows: {rows}");
        println!("hills: {bare}\ngentle: {gentle}\nturned: {turned}\nrows: {rows}");
    }

    #[test]
    fn walls_along_the_axes_follow_the_lattice() {
        let grid = strongest(|seed| world(seed, |_, _| 0.0, walls(true, 0.0)), SEEDS);
        assert!(grid.broken(), "grid: {grid}");
        // Turned an eighth, along the diagonals.
        let diagonal = strongest(|seed| world(seed, |_, _| 0.0, walls(true, PI / 4.0)), SEEDS);
        assert!(diagonal.broken(), "diagonal: {diagonal}");
        println!("grid: {grid}\ndiagonal: {diagonal}");
    }

    #[test]
    fn student_t_tails() {
        // From tables: the 2.5 % and 0.05 % points at 15 degrees of freedom, the 0.05 % at 30, and a quarter at 1.
        for (t, nu, p) in [(2.131, 15.0, 0.025), (4.073, 15.0, 0.0005), (3.646, 30.0, 0.0005), (1.0, 1.0, 0.25), (-2.131, 15.0, 0.975)] {
            let tail = t_tail(t, nu);
            assert!((tail / p - 1.0).abs() < 0.02, "t {t}, {nu} degrees: {tail}, not {p}");
        }
    }
}
