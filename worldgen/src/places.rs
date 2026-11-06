//! Whether things prefer places: a world's layout must not follow the
//! grids it is generated on (cells, plans' regions, tiles), at any seed
//! (docs/generators.md, "Checks"). A town placed once per region, kept a
//! margin inside it, is never on a region's edge; seen over many regions,
//! its positions modulo the region's width bunch up.
//!
//! The test, for each axis and each period P: the things' phases
//! 2π·x/P, summed over blocks of whole periods (a block per seed and
//! stretch of the axis); a preference is a mean of those sums away from
//! zero, against how much they vary (Hotelling's T²). Blocks, not things,
//! are the samples, so things that come in clusters (houses of a town) or
//! bunch where a world has its features (towns along a path) count as
//! much as they repeat: a bias in one stretch is a place, the same bias
//! in every stretch a period. All seeds' things are tested together, to
//! find few that prefer the same places at every seed, and each seed's
//! alone, to find a grid whose phase a seed draws.

use std::collections::HashSet;
use std::f64::consts::TAU;
use std::fmt;

use crate::World;
use crate::generator::{Generator, Layer, Population, anchor_in, made_in};
use crate::grid::Box3;
use crate::noise::{Rng, hash_text};

/** How wide a square around spawn things are gathered in, lattice units (16 km): periods up to 2 km. */
pub const SIDE: i64 = 128_000;
/** Columns of cells gathered in per population and seed, at most, picked at random. */
const COLUMNS: usize = 16_384;
/** Things gathered per population and seed: columns are picked until there are as many. */
const THINGS: usize = 2048;
/** Columns are picked in patches this many a side: neighbours read plans their neighbours made. */
const PATCH: i64 = 2;
/** Stretches each axis is cut into, about: whole periods each, at least one. */
const STRETCHES: i64 = 16;
/** The longest period tested is the extent over this, so it has as many stretches. */
const LONGEST: i64 = 8;
/** The shortest is this many cells, past the lattices' own regularities. */
const SHORTEST: i64 = 4;
/** The chance of a false alarm per population, over all the periods tested. */
pub const ALPHA: f64 = 1e-3;
/** Things a seed needs for a test of its own. */
const ENOUGH: usize = 64;
/** Angles a period is cut into for cosines and sines. */
const ANGLES: usize = 1024;
/** Bins of the histogram a preference shows. */
const BINS: usize = 24;

/**
 * Where the things of `population` are, lattice units: those anchored in
 * `area`, found from its columns of cells over it (along y), all of them
 * if COLUMNS or fewer, else patches of them at random by the world's
 * seed until THINGS are found or COLUMNS picked. A cell's anchor finds
 * the thing of the cell it is in, if any, so a thing many cells ask for
 * (one per region of a plan) is found from any of them.
 * Patches anywhere, not every n-th: a sample on a grid would prefer
 * places itself; and a sample of its own each seed, or its unevenness
 * would be the same at every seed.
 */
pub fn places(generator: &Generator, population: &Population, area: &Box3) -> Vec<[i64; 3]> {
    let n = population.cell;
    let range = |a: usize| ((area.min[a] - n).div_euclid(n), (area.max[a] + n).div_euclid(n));
    let ((i0, i1), (j0, j1), (k0, k1)) = (range(0), range(1), range(2));
    let wide = i1 - i0 + 1;
    let total = (wide * (k1 - k0 + 1)) as u64;
    let column = move |c: u64| (i0 + c as i64 % wide, k0 + c as i64 / wide);
    let all = total <= COLUMNS as u64;
    let columns: Box<dyn Iterator<Item = (i64, i64)>> = if all {
        Box::new((0..total).map(column))
    } else {
        let (mut random, mut seen) = (Rng::new(hash_text(population.name, generator.seed)), HashSet::new());
        let patches = std::iter::from_fn(move || Some(column(random.next_u64() % total)));
        let columns = patches.flat_map(|(i, k)| (0..PATCH * PATCH).map(move |n| (i + n % PATCH, k + n / PATCH)));
        Box::new(columns.filter(move |&c| seen.insert(c)).take(COLUMNS))
    };
    let (mut found, mut asked) = (Vec::new(), HashSet::new());
    for (i, k) in columns {
        for j in (j0..=j1).filter(|j| (i + j + k) & 1 == 0) {
            let Some((_, [i, j, k])) = anchor_in(generator, population, i, j, k) else { continue };
            if asked.insert([i, j, k])
                && let Some((anchor, _)) = made_in(generator, population, i, j, k)
            {
                let p = anchor.map(|v| v.round() as i64);
                if area.holds(p) {
                    found.push(p);
                }
            }
        }
        if !all && found.len() >= THINGS {
            break;
        }
    }
    found
}

/** A population's strongest preference for places in a period, of all it was tested for. */
#[derive(Clone, Debug)]
pub struct Preference {
    pub population: &'static str,
    /** Things gathered, all seeds. */
    pub things: usize,
    /** Tests made: periods along each axis, of all seeds' things and of each seed's alone. */
    pub tests: usize,
    /** The seed whose things alone prefer places most, or none: all seeds' together. */
    pub seed: Option<u64>,
    pub axis: usize,
    pub period: i64,
    /** Where in the period the things bunch: their mean phase, lattice units in [0, period). */
    pub at: i64,
    /** How much: their phases' mean resultant length, 0 (evenly) to 1 (all at `at`). */
    pub concentration: f64,
    /** The chance a preference this strong shows by chance in one of the tests (Bonferroni). */
    pub chance: f64,
    /** Those things in BINS bins of the period. */
    pub histogram: Vec<usize>,
}

impl Preference {
    /** Whether the things prefer places in the period: a chance below ALPHA. */
    pub fn broken(&self) -> bool {
        self.chance < ALPHA
    }
}

impl fmt::Display for Preference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (axis, metres) = (["x", "y", "z"][self.axis], |v: i64| v as f64 * crate::units::UNIT_SIZE);
        let of = self.seed.map_or("all seeds".to_string(), |seed| format!("seed {seed} alone"));
        write!(
            f,
            "{}: {} things, {} tests; most uneven along {axis} modulo {} m ({} units), {of}: \
             bunched at {} m of it, concentration {:.2}, by chance {:.1e}",
            self.population,
            self.things,
            self.tests,
            metres(self.period),
            self.period,
            metres(self.at),
            self.concentration,
            self.chance
        )?;
        if self.broken() {
            write!(f, ": the same places in every period, as if on a grid (see \"Checks\" in docs/generators.md)")?;
        }
        let most = self.histogram.iter().copied().max().unwrap_or(0).max(1);
        let bars: String = self.histogram.iter().map(|&c| [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'][c * 9 / most]).collect();
        write!(f, "\n  {axis} mod {} m, {BINS} bins: |{bars}|", metres(self.period))
    }
}

/** Periods from `shortest` to `longest`, lattice units: every whole one, then as close as the phase drifts a quarter turn over `extent`. */
fn periods(shortest: i64, longest: i64, extent: i64) -> impl Iterator<Item = i64> {
    std::iter::successors(Some(shortest), move |&p| Some(p + (p * p / (4 * extent)).max(1))).take_while(move |&p| p <= longest)
}

/**
 * ln of the chance a mean of `sums` this far from zero shows by chance:
 * Hotelling's T² on the blocks' sums of unit phases, `things` in all,
 * their covariance's eigenvalues at least what independent things' would
 * be (so a lattice, more even than chance, is not taken for a preference).
 */
fn ln_chance(sums: &[[f64; 2]], things: usize) -> f64 {
    let k = sums.len() as f64;
    if sums.len() < 4 || things == 0 {
        return 0.0;
    }
    let m = [0, 1].map(|c| sums.iter().map(|s| s[c]).sum::<f64>() / k);
    let co = |a: usize, b: usize| sums.iter().map(|s| (s[a] - m[a]) * (s[b] - m[b])).sum::<f64>() / (k - 1.0);
    let (a, b, c, m2) = (co(0, 0), co(0, 1), co(1, 1), m[0] * m[0] + m[1] * m[1]);
    // Eigenvalues mid ± r, and m's square along the larger's eigenvector, mᵀ (C − λ₋) m / (λ₊ − λ₋).
    let (mid, r) = ((a + c) / 2.0, ((a - c) / 2.0).hypot(b));
    let along = if r > 0.0 {
        ((a * m[0] * m[0] + 2.0 * b * m[0] * m[1] + c * m[1] * m[1] - (mid - r) * m2) / (2.0 * r)).clamp(0.0, m2)
    } else {
        m2
    };
    // Half a thing to each component, its cosine and sine.
    let floor = things as f64 / k / 2.0;
    let t2 = k * (along / (mid + r).max(floor) + (m2 - along) / (mid - r).max(floor));
    // F = (k − 2) / (2 (k − 1)) T² is F(2, k − 2), whose tail is (1 + 2F / (k − 2))^-(k − 2)/2.
    -(k - 2.0) / 2.0 * (t2 / (k - 1.0)).ln_1p()
}

/**
 * The strongest preference of things for places in a period, along any
 * axis, in `area`, of all seeds' things together or of a seed's alone;
 * `places` holds each seed's things, of a population whose cells are
 * `cell` wide. Periods go from SHORTEST cells to the area's extent over
 * LONGEST.
 */
pub fn preference(population: &'static str, cell: i64, area: &Box3, places: &[(u64, Vec<[i64; 3]>)]) -> Preference {
    let table: Vec<[f64; 2]> = (0..ANGLES).map(|a| TAU * a as f64 / ANGLES as f64).map(|t| [t.cos(), t.sin()]).collect();
    let phase = |x: i64, period: i64| table[(x.rem_euclid(period) * ANGLES as i64 / period) as usize];
    // ln of the chance, axis, period, and the seed (None: all).
    let (mut best, mut tests) = ((0.0, 0, 1, None), 0);
    for axis in 0..3 {
        let extent = area.max[axis] - area.min[axis];
        for period in periods((SHORTEST * cell).max(32), extent / LONGEST, extent) {
            let block = period * (extent / STRETCHES / period).max(1);
            let blocks = (extent / block) as usize;
            let mut sums = vec![[0.0; 2]; places.len() * blocks];
            let mut counts = vec![0; places.len()];
            for (s, (_, things)) in places.iter().enumerate() {
                for p in things {
                    let b = ((p[axis] - area.min[axis]) / block) as usize;
                    if b < blocks {
                        let [c, si] = phase(p[axis], period);
                        sums[s * blocks + b][0] += c;
                        sums[s * blocks + b][1] += si;
                        counts[s] += 1;
                    }
                }
            }
            let alone = places.iter().enumerate().filter(|&(s, _)| counts[s] >= ENOUGH);
            let alone = alone.map(|(s, (seed, _))| (ln_chance(&sums[s * blocks..(s + 1) * blocks], counts[s]), Some(*seed)));
            for (ln, seed) in std::iter::once((ln_chance(&sums, counts.iter().sum()), None)).chain(alone) {
                tests += 1;
                if ln < best.0 {
                    best = (ln, axis, period, seed);
                }
            }
        }
    }
    let (ln, axis, period, seed) = best;
    let (mut sum, mut histogram) = ([0.0; 2], vec![0; BINS]);
    let shown: Vec<i64> =
        places.iter().filter(|(s, _)| seed.is_none_or(|seed| seed == *s)).flat_map(|(_, p)| p.iter().map(|p| p[axis])).collect();
    for &x in &shown {
        let [c, s] = phase(x, period);
        sum = [sum[0] + c, sum[1] + s];
        histogram[(x.rem_euclid(period) * BINS as i64 / period) as usize] += 1;
    }
    Preference {
        population,
        things: places.iter().map(|(_, p)| p.len()).sum(),
        tests,
        seed,
        axis,
        period,
        at: (sum[1].atan2(sum[0]).rem_euclid(TAU) / TAU * period as f64) as i64 % period,
        concentration: sum[0].hypot(sum[1]) / shown.len().max(1) as f64,
        chance: (ln + (tests as f64).ln()).exp().min(1.0),
        histogram,
    }
}

/**
 * Each population's strongest preference for places in a period, its
 * things gathered at each of `seeds` (a thread each) in a square SIDE
 * wide around the first seed's spawn, over the world's heights.
 */
pub fn preferences(make: impl Fn(u64) -> World + Sync, seeds: &[u64]) -> Vec<Preference> {
    fn populations(world: &World) -> impl Iterator<Item = &Population> {
        world.generator.layers.iter().filter_map(|layer| if let Layer::Population(p) = layer { Some(p) } else { None })
    }
    let world = make(seeds[0]);
    let [x, _, z] = world.spawn;
    let area = Box3 { min: [x - SIDE / 2, world.heights.0, z - SIDE / 2], max: [x + SIDE / 2, world.heights.1, z + SIDE / 2] };
    let make = &make;
    std::thread::scope(|scope| {
        let gathering: Vec<_> = seeds
            .iter()
            .map(|&seed| {
                scope.spawn(move || {
                    let world = make(seed);
                    populations(&world).map(|p| places(&world.generator, p, &area)).collect::<Vec<_>>()
                })
            })
            .collect();
        let gathered: Vec<Vec<Vec<[i64; 3]>>> = gathering.into_iter().map(|t| t.join().expect("gathered")).collect();
        let testing: Vec<_> = populations(&world)
            .enumerate()
            .map(|(i, p)| {
                let places: Vec<(u64, Vec<[i64; 3]>)> = seeds.iter().zip(&gathered).map(|(&seed, g)| (seed, g[i].clone())).collect();
                let (name, cell) = (p.name, p.cell);
                scope.spawn(move || preference(name, cell, &area, &places))
            })
            .collect();
        testing.into_iter().map(|t| t.join().expect("tested")).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Box3 = Box3 { min: [-SIDE / 2, 0, -SIDE / 2], max: [SIDE / 2, 8000, SIDE / 2] };

    /** Each of eight seeds' things, `place` picking them with a random sequence of the seed. */
    fn seeds(place: impl Fn(&mut Rng) -> Vec<[i64; 3]>) -> Vec<(u64, Vec<[i64; 3]>)> {
        (0..8).map(|seed| (seed, place(&mut Rng::new(seed)))).collect()
    }

    fn anywhere(random: &mut Rng, a: usize) -> i64 {
        AREA.min[a] + random.below((AREA.max[a] - AREA.min[a]) as u32) as i64
    }

    #[test]
    fn things_anywhere_have_no_preference_however_they_bunch() {
        let random = seeds(|r| (0..2048).map(|_| [0, 1, 2].map(|a| anywhere(r, a))).collect());
        // Clusters of 32 within 40 m of their centres; a band 50 m wide along z, at x = 0 at every seed.
        let clusters = seeds(|r| {
            let centres: Vec<[i64; 3]> = (0..64).map(|_| [0, 1, 2].map(|a| anywhere(r, a))).collect();
            centres.iter().flat_map(|c| (0..32).map(|_| c.map(|v| v - 320 + r.below(640) as i64)).collect::<Vec<_>>()).collect()
        });
        let band = seeds(|r| (0..2048).map(|_| [r.below(400) as i64 - 200, anywhere(r, 1), anywhere(r, 2)]).collect());
        for (what, places) in [("random", random), ("clusters", clusters), ("band", band)] {
            let p = preference("things", 16, &AREA, &places);
            assert!(!p.broken(), "{what}: {p}");
        }
    }

    #[test]
    fn things_kept_inside_regions_prefer_places() {
        // One per region 224 m wide by chance 0.6, kept 40 m inside it, the regions where they are or where each seed puts them.
        const REGION: i64 = 1792;
        let regions = |drawn: bool| {
            seeds(|r| {
                let at = if drawn { [0, 1].map(|_| r.below(REGION as u32) as i64) } else { [0, 0] };
                let regions = (AREA.min[0] / REGION..AREA.max[0] / REGION)
                    .flat_map(|i| (AREA.min[2] / REGION..AREA.max[2] / REGION).map(move |k| (i, k)));
                let inside = |r: &mut Rng, i: i64, at: i64| at + i * REGION + 320 + r.below(REGION as u32 - 640) as i64;
                regions.flat_map(|(i, k)| (r.below(10) < 6).then(|| [inside(r, i, at[0]), anywhere(r, 1), inside(r, k, at[1])])).collect()
            })
        };
        for drawn in [false, true] {
            let p = preference("things", 16, &AREA, &regions(drawn));
            // The regions' width or a harmonic, within a lattice unit or two each.
            assert!(p.broken() && p.axis != 1 && (REGION + 8) % p.period <= 16, "{p}");
        }
        // One every 1200 m along a band, give or take 150 m.
        let towns = seeds(|r| (-6..6).flat_map(|n| (r.below(10) < 6).then(|| [0, 0, n * 9600 - 1200 + r.below(2400) as i64])).collect());
        let p = preference("towns", 2048, &AREA, &towns);
        assert!(p.broken() && p.axis == 2 && (p.period - 9600).abs() <= 9600 / 16, "{p}");
    }
}
