//! The contract every world keeps (docs/generators.md, "Checks"), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; nothing lies outside its heights, things keep their
//! reach, and what is skipped where it says it may not draw draws nothing
//! there; and on average over each stratum's regions, each level shows
//! rays from where a camera could be what the level two finer shows,
//! and seen from above, what level 0 shows. What a ray sees is a `Look`:
//! linear, as a coarse site averages finer ones, so the sum of each
//! kind's, which tell what changed.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::time::Instant;

use fcc::FACE_KINDS;
use fcc::light::srgb_to_linear;

use crate::World;
use crate::checks::survey::{Survey, survey};
use crate::generator::{Generator, Layer, Whole, checking_skipped, generate, materials_of, whole};
use crate::grid::{ALPHA, Box3, VoxelGrid, is_site};
use crate::noise::Rng;
use crate::picture::{Camera, FACES, Seen, Tiles, composite_in, composited, nearest_site};
use crate::units::m;

fn b(min_x: i64, min_y: i64, min_z: i64, max_x: i64, max_y: i64, max_z: i64) -> Box3 {
    Box3 { min: [min_x, min_y, min_z], max: [max_x, max_y, max_z] }
}

/** The box of `level`'s sites covering `region` and a margin of two sites. */
fn box_at(r: &Box3, level: u32) -> Box3 {
    let s = 1i64 << level;
    Box3 { min: r.min.map(|v| v.div_euclid(s) - 2), max: r.max.map(|v| -(-v).div_euclid(s) + 2) }
}

fn sites(b: &Box3) -> impl Iterator<Item = (i64, i64, i64)> + '_ {
    (b.min[2]..b.max[2]).flat_map(move |z| {
        (b.min[1]..b.max[1]).flat_map(move |y| {
            let from = b.min[0] + ((b.min[0] + y + z) & 1);
            (from..b.max[0]).step_by(2).map(move |x| (x, y, z))
        })
    })
}

/**
 * How far a level's look may stray, from the one two finer and from
 * level 0 seen from above, in any channel: a twentieth of the light.
 * On average over a stratum's regions (`strata_kept`); a solid alone,
 * in a region of many (worldgen/tests/shapes.rs).
 */
pub const TOLERANCE: f64 = 0.05;

/** Rays cast at each level, from as many origins. */
const RAYS: usize = 20_000;

/** `region` grown to at least 16 sites of `level` across around its centre, along x and z. */
fn around(region: &Box3, level: u32) -> Box3 {
    let grow = |i: usize| if i == 1 { 0 } else { 0.max((8i64 << level) - (region.max[i] - region.min[i]) / 2) };
    Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) }
}

/**
 * Levels from FAR_FROM see the fields of far regions from above (sites
 * 2 m apart and more), against level 0, within FAR_TOLERANCE.
 */
const FAR_FROM: u32 = 4;

/**
 * How far the fields of a far region may stray from level 0 seen from
 * above, each region. A field is drawn from position alone, and a wide
 * region has hundreds of lines of sites at level 6, so it keeps closer
 * than TOLERANCE, which allows for things' far forms and their surfaces;
 * a plan finer than the sites read where each line is aliases ("Plans"
 * in "Levels" in docs/generators.md).
 */
const FAR_TOLERANCE: f64 = TOLERANCE / 4.0;

/** A box of `region` at `level` generated in eight parts, alone and in any order, equals it generated whole. */
pub fn whole_equals_parts(generator: &Generator, region: &Box3, level: u32) {
    let whole = box_at(region, level);
    let grid = generate(generator, &whole, level);
    assert!([&grid.pieces, &grid.medium].iter().any(|part| part.alpha.iter().any(|a| *a != 0)), "level {level} is empty");
    // Eight parts, split at random points (seeded, so failures reproduce).
    let mut random = Rng::new(7);
    let mut split = |axis: usize, even: bool| {
        let (lo, hi) = (whole.min[axis] as f64, whole.max[axis] as f64);
        let at = lo + (hi - lo) * (0.2 + 0.6 * random.next());
        (if even { 2.0 * (at / 2.0 + 0.5).floor() } else { (at + 0.5).floor() }) as i64
    };
    let (x, y, z) = (split(0, true), split(1, false), split(2, false));
    let mut parts = Vec::new();
    for (min_x, max_x) in [(whole.min[0], x), (x, whole.max[0])] {
        for (min_y, max_y) in [(whole.min[1], y), (y, whole.max[1])] {
            for (min_z, max_z) in [(whole.min[2], z), (z, whole.max[2])] {
                parts.push(b(min_x, min_y, min_z, max_x, max_y, max_z));
            }
        }
    }
    for part in parts.iter().rev() {
        let alone = generate(generator, part, level);
        for (x, y, z) in sites(part) {
            // What shows, and what rays and light see: its alpha, each face's material, its glow.
            let look = |g: &VoxelGrid| {
                (g.get(x, y, z), g.faces_at(x, y, z).copied(), std::array::from_fn::<_, FACE_KINDS, _>(|face| g.light(x, y, z, face)))
            };
            assert_eq!(look(&alone), look(&grid), "level {level}: part {part:?} differs at ({x}, {y}, {z})");
        }
    }
}

/**
 * The empty sites of a grid a camera could be at: those it could fly to
 * from above, connected to the top of the grid's box through empty sites
 * (voxels sharing a face): empty as shown, as a camera collides with the
 * voxels shown, not with what stops some rays. Under the ground's shell,
 * inside a closed house or a boulder are not; under a canopy, open to
 * the sky somewhere, is.
 */
struct Open {
    b: Box3,
    /** By point of `b` and a point more on each side, x fastest: 0 not reached yet, 1 open, 2 full or outside. */
    state: Vec<u8>,
}

impl Open {
    fn new(grid: &VoxelGrid) -> Open {
        let b = grid.clip();
        let n = [0, 1, 2].map(|i| b.max[i] - b.min[i] + 2);
        let mut state = vec![2u8; (n[0] * n[1] * n[2]) as usize];
        let at = |p: [i64; 3]| ((p[0] - b.min[0] + 1) + n[0] * ((p[1] - b.min[1] + 1) + n[1] * (p[2] - b.min[2] + 1))) as usize;
        for (x, y, z) in sites(&b) {
            if grid.get(x, y, z) == 0 {
                state[at([x, y, z])] = 0;
            }
        }
        let mut stack = Vec::new();
        for z in b.min[2]..b.max[2] {
            for x in b.min[0]..b.max[0] {
                // The top site of the column (x, z): in one of the top two layers.
                let y = b.max[1] - 1 - (x + b.max[1] - 1 + z).rem_euclid(2);
                if y >= b.min[1] && state[at([x, y, z])] == 0 {
                    state[at([x, y, z])] = 1;
                    stack.push(at([x, y, z]));
                }
            }
        }
        // Neighbours by their offset in `state`: the points outside the box are full.
        let faces = FACES.map(|f| f[0] + n[0] * (f[1] + n[1] * f[2]));
        while let Some(i) = stack.pop() {
            for face in faces {
                let q = (i as i64 + face) as usize;
                if state[q] == 0 {
                    state[q] = 1;
                    stack.push(q);
                }
            }
        }
        Open { b, state }
    }

    fn holds(&self, p: [i64; 3]) -> bool {
        let (b, n) = (&self.b, [0, 1, 2].map(|i| self.b.max[i] - self.b.min[i] + 2));
        b.holds(p)
            && is_site(p[0], p[1], p[2])
            && self.state[((p[0] - b.min[0] + 1) + n[0] * ((p[1] - b.min[1] + 1) + n[1] * (p[2] - b.min[2] + 1))) as usize] == 1
    }
}

struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
    /** How far it goes in the area it is cast in, lattice units. */
    end: f64,
}

/** RAYS rays in random directions from random sites of `area` a camera could be at in `reference` (at `level`), seeded so failures reproduce; and how many origins were tried for them. */
fn rays_from_open(area: &Box3, reference: &VoxelGrid, level: u32) -> (Vec<Ray>, usize) {
    let open = Open::new(reference);
    let s = (1i64 << level) as f64;
    let mut random = Rng::new(11);
    let (mut rays, mut tried) = (Vec::with_capacity(RAYS), 0);
    while rays.len() < RAYS && tried < 50 * RAYS {
        tried += 1;
        let origin: [f64; 3] = std::array::from_fn(|i| area.min[i] as f64 + random.next() * (area.max[i] - area.min[i]) as f64);
        let z = 2.0 * random.next() - 1.0;
        let a = 2.0 * std::f64::consts::PI * random.next();
        let direction = [(1.0 - z * z).sqrt() * a.cos(), (1.0 - z * z).sqrt() * a.sin(), z];
        if !open.holds(nearest_site(origin.map(|v| v / s))) {
            continue;
        }
        // Where it leaves the area.
        let end = (0..3)
            .map(|i| match direction[i] {
                d if d > 0.0 => (area.max[i] as f64 - origin[i]) / d,
                d if d < 0.0 => (area.min[i] as f64 - origin[i]) / d,
                _ => f64::INFINITY,
            })
            .fold(f64::INFINITY, f64::min);
        rays.push(Ray { origin, direction, end });
    }
    (rays, tried)
}

/** What a ray sees of a grid (`composited`), its distances along it in lattice units. */
type Path = Vec<(f64, u8, f64, [f64; 3])>;

/**
 * Each ray's path through `grid` (at `level`), from `from` along it till
 * it leaves its area, its distances from there (lattice units): all of
 * what gets to `from` taken as the ray.
 */
fn paths(grid: &VoxelGrid, level: u32, rays: &[Ray], from: f64) -> Vec<Path> {
    let s = (1i64 << level) as f64;
    let path = |ray: &Ray| {
        if ray.end <= from {
            return Vec::new();
        }
        let origin: [f64; 3] = std::array::from_fn(|i| (ray.origin[i] + from * ray.direction[i]) / s);
        composited(grid, origin, ray.direction, (ray.end - from) / s)
    };
    rays.iter().map(|ray| path(ray).into_iter().map(|(t, id, stopped, glow)| (t * s, id, stopped, glow)).collect()).collect()
}

/** How much of a ray gets past `t` along `path` (lattice units). */
fn through_to(path: &Path, t: f64) -> f64 {
    1.0 - path.iter().take_while(|(at, ..)| *at <= t).map(|(_, _, stopped, _)| stopped).sum::<f64>()
}

/** By material id, its kind (index in Generator::kinds; none for id 0) and its colour, linear. */
struct Palette {
    kinds: Vec<Option<usize>>,
    colours: Vec<[f64; 3]>,
}

impl Palette {
    fn of(generator: &Generator) -> Palette {
        // Kinds have consecutive material ids, one per colour, after 0 (none): see materials_of.
        let mut kinds = vec![None];
        for (i, kind) in generator.kinds.iter().enumerate() {
            kinds.extend(kind.colors.iter().map(|_| Some(i)));
        }
        let colours = materials_of(generator)
            .iter()
            .map(|m| [16, 8, 0].map(|shift| srgb_to_linear(((m.rgb() >> shift) & 255) as f32 / 255.0) as f64))
            .collect();
        Palette { kinds, colours }
    }

    /** Adds to `looks` what id `id` stops of a ray (`stopped`), near or not, and what it glows to it. */
    fn add(&self, looks: &mut Looks, id: u8, near: bool, stopped: f64, glow: [f64; 3]) {
        let look = looks.entry(self.kinds[id as usize]).or_default();
        look[if near { 0 } else { 1 }] += stopped;
        let colour = if id == 0 { [0.0; 3] } else { self.colours[id as usize] };
        (0..3).for_each(|c| look[SHOWN.start + c] += stopped * colour[c] + glow[c]);
    }
}

/**
 * What rays see, a mean over them, linear, as under a white light all
 * round: how much of them stops near (`STOPPED`'s first channel) and
 * further (its second), and what shows (`SHOWN`): each site's colour by
 * how much of them it stops, and what glows to them, premultiplied as
 * compositing has it. With no ratio, a few rays of many change it by no
 * more than their share; with no sky behind, over any sky it differs by
 * no more than its channels do. Linear, it is the sum of what each kind
 * stops and shows (`Looks`).
 */
pub type Look = [f64; 5];

/** A look by kind: its index in Generator::kinds, none for what stops with no material. */
pub type Looks = BTreeMap<Option<usize>, Look>;

/** The channels of a `Look` that say how much stops: near, and further. */
pub const STOPPED: Range<usize> = 0..2;

/** The channels of a `Look` that say what shows: red, green and blue. */
pub const SHOWN: Range<usize> = 2..5;

/** How far a camera's rays stop "near", past where a level is looked at from: in the level's sites. */
const NEAR: f64 = 4.0;

/**
 * Adds to `looks` what a ray sees along `path`, by `weight` (the light
 * that gets to its start): stopping within `near` of its start (lattice
 * units) in the first channel.
 */
fn add(looks: &mut Looks, path: &[(f64, u8, f64, [f64; 3])], near: f64, weight: f64, palette: &Palette) {
    for &(t, id, stopped, glow) in path {
        palette.add(looks, id, t <= near, weight * stopped, glow.map(|g| weight * g));
    }
}

/** The looks of what pixels see (a mean over them): what stops, all of it near. */
fn looks_of(seen: &Seen, palette: &Palette) -> Looks {
    let mut looks = Looks::new();
    for (&id, &stopped) in &seen.stopped {
        palette.add(&mut looks, id, true, stopped, [0.0; 3]);
    }
    for (&id, &glow) in &seen.glow {
        palette.add(&mut looks, id, true, 0.0, glow);
    }
    looks
}

/** What a margin measures. */
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Measure {
    /** What rays from where a camera could be see past 2 sites of the level. */
    Cameras,
    /** What rays straight down see. */
    FromAbove,
    /** What rays straight down see of the fields alone. */
    FieldsFromAbove,
}

/** How far a level of a world strays from another in a region, by one measure. */
#[derive(Clone, Debug)]
pub struct Margin {
    pub region: Box3,
    pub measure: Measure,
    pub level: u32,
    /** The level it is measured against: two finer, or 0 from above. */
    pub against: u32,
    /** What rays see, by kind, at `against` and at `level`. */
    pub looks: [Looks; 2],
}

impl Margin {
    /** What rays see of every kind, at `against` and at `level`. */
    pub fn values(&self) -> [Look; 2] {
        self.looks.each_ref().map(|looks| looks.values().fold([0.0; 5], |sum, l| std::array::from_fn(|i| sum[i] + l[i])))
    }

    /** The kinds rays see, at either level: to tell what changed, not checked, as levels may draw a kind for another. */
    fn kinds(&self) -> BTreeSet<Option<usize>> {
        self.looks.iter().flat_map(|looks| looks.keys().copied()).collect()
    }

    /** How far `kind`'s look at `level` is from its look at `against`, channel by channel. */
    fn change(&self, kind: Option<usize>) -> Look {
        let [a, c] = self.looks.each_ref().map(|looks| looks.get(&kind).copied().unwrap_or_default());
        std::array::from_fn(|i| c[i] - a[i])
    }

    /** How far apart the looks are in `channels`: in the one they differ the most in. */
    pub fn apart(&self, channels: Range<usize>) -> f64 {
        let [a, c] = self.values();
        channels.map(|i| (c[i] - a[i]).abs()).fold(0.0, f64::max)
    }

    /** How far apart the looks are, in any channel. */
    pub fn difference(&self) -> f64 {
        self.apart(0..5)
    }

    /** Whether this region alone is as far apart as a stratum may be on average, or more. */
    pub fn broken(&self) -> bool {
        self.difference() >= self.tolerance()
    }

    /** How far it may be apart. */
    pub fn tolerance(&self) -> f64 {
        if self.measure == Measure::FieldsFromAbove { FAR_TOLERANCE } else { TOLERANCE }
    }
}

/**
 * A world's margins in its regions, to print: `{}` breaks down the broken
 * ones by kind, `{:#}` all of them.
 */
#[derive(Clone)]
pub struct Report {
    pub name: String,
    pub margins: Vec<Margin>,
    /** What regions hold, to print after them: the strata `check` made them for. */
    pub held: Vec<(Box3, String)>,
    /** The kinds' names, by index in Generator::kinds. */
    kinds: Vec<&'static str>,
}

impl Report {
    fn new(name: &str, generator: &Generator) -> Report {
        Report { name: name.to_string(), margins: Vec::new(), held: Vec::new(), kinds: generator.kinds.iter().map(|k| k.name).collect() }
    }

    /** Only the broken margins. */
    pub fn broken(&self) -> Report {
        Report { margins: self.margins.iter().filter(|m| m.broken()).cloned().collect(), ..self.clone() }
    }

    /** A kind's name; "none" for what stops with no material. */
    fn kind(&self, kind: Option<usize>) -> &'static str {
        kind.map_or("none", |k| self.kinds[k])
    }

    /** A margin's looks by kind, largest changes first, but those under 0.005 at both levels. */
    fn breakdown(&self, f: &mut std::fmt::Formatter, m: &Margin) -> std::fmt::Result {
        let largest = |l: Look| l.iter().fold(0.0, |a: f64, v| a.max(v.abs()));
        let mut kinds: Vec<_> = m.kinds().into_iter().collect();
        kinds.sort_by(|a, b| largest(m.change(*b)).total_cmp(&largest(m.change(*a))));
        for kind in kinds {
            let [a, c] = m.looks.each_ref().map(|looks| looks.get(&kind).copied().unwrap_or_default());
            if largest(a).max(largest(c)) >= 0.005 {
                writeln!(f, "      {}: {} -> {}: {:.3} apart", self.kind(kind), look(&a), look(&c), largest(m.change(kind)))?;
            }
        }
        Ok(())
    }
}

/** A look, to print: how much stops, near + further, and what shows. */
fn look(l: &Look) -> String {
    format!("stops {:.3} + {:.3}, shows {:.3} {:.3} {:.3}", l[0], l[1], l[2], l[3], l[4])
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut region = None;
        for m in &self.margins {
            if region != Some(m.region) {
                let r = m.region;
                let held = self.held.iter().find(|(b, _)| *b == r).map_or(String::new(), |(_, h)| format!(" {h}"));
                writeln!(
                    f,
                    "{} in x {}..{}, y {}..{}, z {}..{}:{held}",
                    self.name, r.min[0], r.max[0], r.min[1], r.max[1], r.min[2], r.max[2]
                )?;
                region = Some(r);
            }
            let what = match m.measure {
                Measure::Cameras => "cameras",
                Measure::FromAbove => "from above",
                Measure::FieldsFromAbove => "fields from above",
            };
            let [a, c] = m.values().map(|v| look(&v));
            let broken = if m.broken() { format!(", over {}", m.tolerance()) } else { String::new() };
            writeln!(f, "  level {} against {}, {what}: {a} -> {c}: {:.3} apart{broken}", m.level, m.against, m.difference())?;
            if m.broken() || f.alternate() {
                self.breakdown(f, m)?;
            }
        }
        Ok(())
    }
}

/**
 * What rays from where a camera could be see of `region` at each level
 * from 1 to `levels` (a `Look`): the level two finer near, to 2 of the
 * level's sites, and the level past them, as hosts draw a coarse level
 * behind a finer one; against the level two finer all the way.
 */
fn levels_against_finer(generator: &Generator, region: &Box3, levels: u32, report: &mut Report) {
    let palette = Palette::of(generator);
    // Grids generated, by level and box: a level's is the reference of the level two coarser.
    let mut grids: Vec<(u32, Box3, VoxelGrid)> = Vec::new();
    let grid = |grids: &mut Vec<(u32, Box3, VoxelGrid)>, level: u32, area: &Box3| -> usize {
        let b = box_at(area, level);
        grids.iter().position(|(l, g, _)| *l == level && *g == b).unwrap_or_else(|| {
            grids.push((level, b, generate(generator, &b, level)));
            grids.len() - 1
        })
    };
    let mut origins: Option<(u32, Box3, Vec<Ray>)> = None;
    for level in 1..=levels {
        let (s, finer) = ((1i64 << level) as f64, level.saturating_sub(2));
        // At least 16 sites of the level across and tall, around the region's centre.
        let mut area = around(region, level);
        let grow = 0.max((8i64 << level) - (region.max[1] - region.min[1]) / 2);
        (area.min[1], area.max[1]) = (region.min[1] - grow, region.max[1] + grow);
        let reference = grid(&mut grids, finer, &area);
        if origins.as_ref().is_none_or(|(l, a, _)| *l != finer || *a != area) {
            let (rays, tried) = rays_from_open(&area, &grids[reference].2, finer);
            assert!(rays.len() >= RAYS / 2, "level {level}: a camera could be at only {} of {tried} points", rays.len());
            origins = Some((finer, area, rays));
        }
        let rays = &origins.as_ref().expect("made").2;
        let from = 2.0 * s;
        let fine = paths(&grids[reference].2, finer, rays, 0.0);
        let coarse_grid = grid(&mut grids, level, &area);
        let coarse = paths(&grids[coarse_grid].2, level, rays, from);
        let mut looks = [Looks::new(), Looks::new()];
        for (fine, coarse) in fine.iter().zip(&coarse) {
            // Past `from`, at each level, of the light that gets there at the finer.
            let past: Path = fine.iter().filter(|(t, ..)| *t > from).map(|&(t, id, stopped, glow)| (t - from, id, stopped, glow)).collect();
            add(&mut looks[0], &past, NEAR * s, 1.0, &palette);
            add(&mut looks[1], coarse, NEAR * s, through_to(fine, from), &palette);
        }
        let looks = looks.map(|looks| looks.into_iter().map(|(k, l)| (k, l.map(|v| v / rays.len() as f64))).collect());
        report.margins.push(Margin { region: *region, measure: Measure::Cameras, level, against: finer, looks });
        // Only the grids the coming levels read: the coarse one, and the one after it.
        grids.retain(|(l, ..)| *l + 1 >= level);
    }
}

/**
 * What a camera straight above `region`, grown to at least 16 sites of
 * the level across around its centre, sees at each level from 1 to
 * `levels`: what rays down see (a `Look`), as at level 0. Unlike levels
 * two apart, level 0 catches a drift, a little a level, such as walls
 * taking over from roofs.
 */
fn from_above_against_level_0(world: &World, region: &Box3, levels: u32, report: &mut Report) {
    from_above(world, region, 1..=levels, Measure::FromAbove, report);
}

/** What rays straight down over `region` see at each of `levels`, against level 0, as `measure`. */
fn from_above(world: &World, region: &Box3, levels: impl Iterator<Item = u32>, measure: Measure, report: &mut Report) {
    let palette = Palette::of(&world.generator);
    let seen = |tiles: &mut Tiles, area: &Box3| {
        // A ray every 2 units.
        let width = ((area.max[0] - area.min[0]) / 2).clamp(16, 1024) as usize;
        let camera = Camera::top([area.min[0] as f64, area.min[2] as f64], [area.max[0] as f64, area.max[2] as f64], width);
        composite_in(&world.generator, &camera, tiles)
    };
    // Level 0 over the area of the levels since it last grew; the areas
    // nest, so its tiles are kept for the next.
    let mut level_0 = Tiles::new(0, world.heights_at(0));
    let mut reference: Option<(Box3, Seen)> = None;
    for level in levels {
        let area = around(region, level);
        if reference.as_ref().is_none_or(|(a, _)| *a != area) {
            reference = Some((area, seen(&mut level_0, &area)));
        }
        let reference = &reference.as_ref().expect("made").1;
        let here = seen(&mut Tiles::new(level, world.heights_at(level)), &area);
        let looks = [looks_of(reference, &palette), looks_of(&here, &palette)];
        report.margins.push(Margin { region: *region, measure, level, against: 0, looks });
    }
}

/**
 * How far each level of `world`, named `name` (with its seed, say), up
 * to `levels` strays in each of `regions` from the level two finer (rays
 * from where a camera could be), and up to `from_above` from level 0
 * (seen from above).
 */
pub fn margins(name: &str, world: &World, regions: &[Box3], levels: u32, from_above: u32) -> Report {
    let mut report = Report::new(name, &world.generator);
    for region in regions {
        levels_against_finer(&world.generator, region, levels, &mut report);
        from_above_against_level_0(world, region, from_above, &mut report);
    }
    report
}

/**
 * What `world`'s fields show from above over each of `regions`, wide
 * ones (only x and z count), at levels FAR_FROM to `levels`, against
 * level 0. Its things are left out, and with them their far
 * fields: their far forms keep what rays see around them, not from
 * above ("Media are isotropic" in docs/generators.md).
 */
pub fn far_margins(name: &str, world: World, regions: &[Box3], levels: u32) -> Report {
    let world = fields_of(world);
    let mut report = Report::new(name, &world.generator);
    for region in regions {
        from_above(&world, region, FAR_FROM..=levels, Measure::FieldsFromAbove, &mut report);
    }
    report
}

/** `far_margins`, printed; panics where one breaks FAR_TOLERANCE. */
pub fn check_far(name: &str, world: World, regions: &[Box3], levels: u32) {
    let start = Instant::now();
    let report = far_margins(name, world, regions, levels);
    println!("{report}{name}: far {:.1} s", start.elapsed().as_secs_f64());
    let broken = report.broken();
    assert!(broken.margins.is_empty(), "{name} breaks the contract far away:\n{broken}");
}

/** Sites of each level above a world's heights that `heights_hold` generates. */
const BEYOND: i64 = 16;

/** The kind of material `id` of `generator`, for messages. */
fn kind_of(generator: &Generator, id: u8) -> &'static str {
    let mut first = 1;
    for kind in &generator.kinds {
        first += kind.colors.len();
        if (id as usize) < first {
            return kind.name;
        }
    }
    "none"
}

/** Where a level `level` box of `grid` has a site that stops any light (its alpha), in lattice units, and its kind; how many it has. */
fn stopping(generator: &Generator, grid: &VoxelGrid, level: u32) -> Option<([i64; 3], &'static str, usize)> {
    let b = grid.clip();
    let full: Vec<_> = sites(&b).filter(|&(x, y, z)| grid.alpha(x, y, z) != 0).collect();
    let &(x, y, z) = full.first()?;
    let (_, id) = grid.light(x, y, z, 0).0.into_iter().find(|part| part.0 != 0)?;
    Some(([x, y, z].map(|v| v << level), kind_of(generator, id), full.len()))
}

/**
 * Hosts generate a world only within its heights, so nothing may lie
 * over them: at each level up to one past `levels`, in each region and a
 * host tile's width around spawn, BEYOND sites above the heights are
 * empty. Under them the ground may go on (Field::under), unseen. Returns
 * where they are not.
 */
pub fn heights_hold(world: &World, regions: &[Box3], levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    for level in 0..=levels + 1 {
        let high = world.heights_at(level).1;
        // A host's tile: 128 sites wide.
        let half = 64 << level;
        let [x, _, z] = world.spawn;
        let spawn = Box3 { min: [x - half, 0, z - half], max: [x + half, 0, z + half] };
        for (i, area) in regions.iter().chain([&spawn]).enumerate() {
            let b = box_at(area, level);
            let slab = Box3 { min: [b.min[0], high, b.min[2]], max: [b.max[0], high + BEYOND, b.max[2]] };
            if let Some((at, kind, n)) = stopping(&world.generator, &generate(&world.generator, &slab, level), level) {
                let place = if i == regions.len() { "around spawn".to_string() } else { format!("in {area:?}") };
                broken.push(format!(
                    "level {level}, {place}: {n} sites stopping light above the heights {:?} (World::heights), which hosts don't generate, \
                     such as {kind} at {at:?} lattice units",
                    world.heights
                ));
            }
        }
    }
    broken
}

/** Sites a side of the boxes `skips_hold` generates: small, so lines and things are skipped near what they draw. */
const SKIPPED: i64 = 32;

/**
 * What is skipped where it says it may not draw (a field's lines about
 * their ground, a population standing on it) draws nothing there, and
 * what a field fills under its ground without asking is what it draws:
 * in each region, at the levels `whole_equals_parts` checks, cut into
 * boxes SKIPPED sites a side, each generated with what it skips drawn all
 * the same (`checking_skipped`). And a box `whole` says holds nothing
 * holds nothing, one it says is buried is solid, a site around it too.
 * Returns where they do not.
 */
pub fn skips_hold(world: &World, regions: &[Box3], levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    for level in [0, 2, levels + 1] {
        for region in regions {
            let b = box_at(region, level);
            let starts = |a: usize| (b.min[a]..b.max[a]).step_by(SKIPPED as usize);
            for z in starts(2) {
                for y in starts(1) {
                    for x in starts(0) {
                        let part = Box3 { min: [x, y, z], max: [x + SKIPPED, y + SKIPPED, z + SKIPPED] }.intersect(&b);
                        let (grid, drew) = checking_skipped(|| generate(&world.generator, &part.widen(1), level));
                        broken.extend(drew.into_iter().map(|d| format!("level {level}: {d}")));
                        let Some(whole) = whole(&world.generator, &part, level) else { continue };
                        let sites = part.widen(1);
                        let sites = (sites.min[2]..sites.max[2])
                            .flat_map(|z| {
                                (sites.min[1]..sites.max[1]).flat_map(move |y| (sites.min[0]..sites.max[0]).map(move |x| [x, y, z]))
                            })
                            .filter(|&[x, y, z]| is_site(x, y, z));
                        let unlike = match whole {
                            Whole::Empty => sites.filter(|&p| part.holds(p)).find(|&[x, y, z]| grid.alpha(x, y, z) > 0),
                            Whole::Buried => sites.into_iter().find(|&[x, y, z]| grid.alpha(x, y, z) < ALPHA),
                        };
                        if let Some(site) = unlike {
                            broken.push(format!("level {level}: {part:?} is {whole:?}, as its layers say, but not at site {site:?}"));
                        }
                    }
                }
            }
        }
    }
    broken.sort();
    broken.dedup();
    broken
}

/**
 * The things of each population of `world` that `survey` found, of all
 * heights: each keeps within its population's reach (which `generate`
 * checks only where it draws), and where its bounds leave the world's
 * heights, nothing is generated outside them (at level 0). Returns where
 * they do not.
 */
pub fn things_hold(world: &World, survey: &Survey) -> Vec<String> {
    let mut broken = Vec::new();
    let (h0, h1) = world.heights;
    for things in &survey.things {
        for (key, bounds) in &things.leaving {
            for (from, to, side) in [(h1, bounds.max[1] + 2, "above"), (bounds.min[1] - 2, h0, "below")] {
                if from >= to {
                    continue;
                }
                let b = Box3 { min: [bounds.min[0] - 2, from, bounds.min[2] - 2], max: [bounds.max[0] + 2, to, bounds.max[2] + 2] };
                if let Some((at, kind, n)) = stopping(&world.generator, &generate(&world.generator, &b, 0), 0) {
                    broken.push(format!(
                        "{key}: {n} sites stopping light {side} the heights {:?} (World::heights), which hosts don't generate, \
                         such as {kind} at {at:?} lattice units",
                        world.heights
                    ));
                }
            }
        }
        let Some((key, bounds, allowed)) = things.past.first() else { continue };
        let over = |b: &Box3, a: &Box3, i: usize| 0.max(a.min[i] - b.min[i]).max(b.max[i] - a.max[i]);
        let needed: [i64; 3] = [0, 1, 2].map(|i| things.reach[i] + things.past.iter().map(|(_, b, a)| over(b, a, i)).max().unwrap_or(0));
        broken.push(format!(
            "{} of the {} {} surveyed reach past their population's reach {:?}; it needs at least {needed:?} \
             (see \"Populations\" in docs/generators.md); such as {key}: bounds {bounds:?}, reach allows {allowed:?}",
            things.past.len(),
            things.made,
            things.population,
            things.reach,
        ));
    }
    broken
}

/** `world` with its fields alone: its things left out, and with them their far fields. */
fn fields_of(mut world: World) -> World {
    world.generator.layers.retain(|layer| matches!(layer, Layer::Field(_)));
    world
}

/**
 * The widest region the contract checks in (32 m), along any axis: each
 * holds places of strata (worldgen::checks::survey), as many as fit.
 */
pub const REGION: i64 = m(32.0);

/**
 * What `world` is made of (`survey`), and regions holding a place of
 * each of its strata, with their names; `only` those of strata whose
 * names hold one of `only`, if any (a layer's name first: "towns ›
 * church", "railway"), and only their layers surveyed.
 */
pub fn regions(world: &World, only: &[&str]) -> (Survey, Vec<(Box3, Vec<String>)>) {
    let survey = survey(world, REGION, |layer| only.is_empty() || only.iter().any(|o| o.starts_with(layer) || layer.starts_with(o)));
    let names: Vec<String> = (0..survey.strata.len()).map(|i| survey.describe(&world.generator, i)).collect();
    let wanted = |i: &usize| only.is_empty() || only.iter().any(|o| names[*i].contains(o));
    let regions = survey.regions(wanted).into_iter().map(|(b, strata)| (b, strata.iter().map(|i| names[*i].clone()).collect())).collect();
    (survey, regions)
}

/** What `check` found of a world at one seed. */
pub struct Checked {
    pub name: String,
    /** Each region and the strata it holds. */
    pub regions: Vec<(Box3, Vec<String>)>,
    /** The margins of every region. */
    pub report: Report,
    /** Where it breaks the rules that hold exactly: its heights, things' reach, what is skipped. */
    pub broken: Vec<String>,
    /** The layers of its generator that draw nothing surveyed, by name: none checked, if `complete`. */
    pub silent: Vec<&'static str>,
    /** Whether the regions hold every stratum, not `only` some. */
    pub complete: bool,
}

/** A margin, and the world checked it is of. */
type Held<'a> = (&'a Checked, &'a Margin);

/**
 * Where worlds checked (at several seeds, say) break the contract: the
 * rules that hold exactly, layers that draw nothing, and each stratum's
 * margins (by measure and level) averaged over the regions holding it
 * at every seed, if they are TOLERANCE apart or more in any channel.
 * Each broken mean names the regions it was averaged over, and the
 * worst of them broken down by kind.
 */
pub fn strata_kept(checked: &[Checked]) -> Vec<String> {
    let mut broken = Vec::new();
    for c in checked {
        broken.extend(c.broken.iter().map(|b| format!("{}: {b}", c.name)));
        if c.complete && !c.silent.is_empty() {
            broken.push(format!(
                "{}: {} draw nothing around spawn or the places it names (worldgen::checks::survey): too far from them, or not at all",
                c.name,
                c.silent.join(", ")
            ));
        }
    }
    // Each stratum's margins, by measure and level, from every world checked.
    let mut means: BTreeMap<(&str, Measure, u32), Vec<Held>> = BTreeMap::new();
    for c in checked {
        for m in &c.report.margins {
            let Some((_, strata)) = c.regions.iter().find(|(b, _)| *b == m.region) else { continue };
            strata.iter().for_each(|s| means.entry((s, m.measure, m.level)).or_default().push((c, m)));
        }
    }
    for ((stratum, measure, level), margins) in means {
        let n = margins.len() as f64;
        let changes = margins.iter().map(|(_, m)| {
            let [a, c] = m.values();
            std::array::from_fn::<f64, 5, _>(|i| c[i] - a[i])
        });
        let mean = changes.fold([0.0; 5], |sum, c| std::array::from_fn(|i| sum[i] + c[i] / n));
        let apart = mean.iter().fold(0.0, |a: f64, v| a.max(v.abs()));
        if apart < TOLERANCE {
            continue;
        }
        let mut message = format!(
            "{stratum}, level {level}, {measure:?}: {apart:.3} apart on average over {} regions (stops {:+.3} + {:+.3}, shows {:+.3} {:+.3} {:+.3}):",
            margins.len(),
            mean[0],
            mean[1],
            mean[2],
            mean[3],
            mean[4]
        );
        for (c, m) in &margins {
            message += &format!("\n  {} in {:?}: {:.3}", c.name, m.region, m.difference());
        }
        let (c, worst) = margins.iter().max_by(|a, b| a.1.difference().total_cmp(&b.1.difference())).expect("a margin");
        message += &format!("\n{:#}", Report { margins: vec![(*worst).clone()], ..c.report.clone() });
        broken.push(message);
    }
    broken
}

/** Panics where worlds checked (at several seeds, say) break the contract (`strata_kept`). */
pub fn assert_kept(checked: &[Checked]) {
    let broken = strata_kept(checked);
    assert!(broken.is_empty(), "the contract is broken:\n{}", broken.join("\n"));
}

/**
 * The whole contract for `world`, named `name` (with its seed, say), in
 * regions holding everything it is made of around spawn (`regions`),
 * `only` some if any, up to the level `levels` it keeps what a camera
 * sees to, and `from_above` what is seen from above; and with `parts`,
 * each region generated in parts. Prints its regions and margins (see
 * them with --nocapture); returns where it breaks it.
 */
pub fn check(name: &str, world: &World, levels: u32, from_above: u32, parts: bool, only: &[&str]) -> Checked {
    let start = Instant::now();
    let (survey, regions) = regions(world, only);
    let layers: Vec<String> = survey
        .layers
        .iter()
        .map(|l| format!("{} {:.1} km {:.1} s", l.layer, 2.0 * l.half as f64 * crate::units::UNIT_SIZE / 1000.0, l.seconds))
        .collect();
    let (n, took) = (survey.strata.len(), start.elapsed().as_secs_f64());
    println!("{name}: {n} strata in {} regions, surveyed in {took:.1} s: {}", regions.len(), layers.join(", "));
    if only.is_empty() {
        // Not checked: a world may share a table of kinds with others (the moor's is the valley's).
        println!("{name}: kinds nothing surveyed draws: {}", survey.unseen(&world.generator).join(", "));
    }
    let boxes: Vec<Box3> = regions.iter().map(|(b, _)| *b).collect();
    let mut broken = Vec::new();
    if parts {
        let start = Instant::now();
        for region in &boxes {
            // Up to one level past the checked ones: where things may be drawn as media (a forest's trees).
            for level in [0, 2, levels + 1] {
                whole_equals_parts(&world.generator, region, level);
            }
        }
        println!("{name}: parts {:.1} s", start.elapsed().as_secs_f64());
    }
    let start = Instant::now();
    let report = Report {
        held: regions.iter().map(|(b, strata)| (*b, strata.join(", "))).collect(),
        ..margins(name, world, &boxes, levels, from_above)
    };
    let rays = start.elapsed().as_secs_f64();
    broken.extend(heights_hold(world, &boxes, levels));
    let heights = start.elapsed().as_secs_f64() - rays;
    broken.extend(things_hold(world, &survey));
    let things = start.elapsed().as_secs_f64() - rays - heights;
    broken.extend(skips_hold(world, &boxes, levels));
    let skips = start.elapsed().as_secs_f64() - rays - heights - things;
    println!("{report}{name}: margins {rays:.1} s, heights {heights:.1} s, things {things:.1} s, skips {skips:.1} s");
    let silent = survey.layers.iter().filter(|l| l.found == 0).map(|l| l.layer).collect();
    Checked { name: name.to_string(), regions, report, broken, silent, complete: only.is_empty() }
}
