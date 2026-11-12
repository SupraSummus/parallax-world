//! The contract every world keeps (docs/generators.md, "Checks"), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; each level shows rays from where a camera could be
//! what the level two finer shows; seen from above, what level 0 shows;
//! nothing lies outside its heights, things keep their reach, and fields
//! draw nowhere they say they may not (Field::within). Rays
//! see each site by its alpha, through what is in front of it: its
//! colour and its glow, which average, as a coarse site stands for finer
//! ones; not its material, which doesn't.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use bake::FACE_KINDS;
use bake::light::{linear_to_srgb, srgb_to_linear};

use crate::World;
use crate::generator::{Generator, Layer, checking_within, generate, materials_of};
use crate::grid::{Box3, VoxelGrid, is_site};
use crate::noise::Rng;
use crate::picture::{Camera, FACES, Seen, Tiles, composite_in, composited, nearest_site};
use crate::survey::{Survey, survey};
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

/** How far a level may stray from the one two finer in the share of rays through (docs/generators.md). */
const THROUGH_TOLERANCE: f64 = 0.1;

/**
 * How far a level's colours may stray, from the one two finer and from
 * level 0 seen from above: in any channel of what rays see (`colour`).
 */
const COLOUR_TOLERANCE: f64 = 0.1;

/** Rays cast at each level, from as many origins. */
const RAYS: usize = 20_000;

/** `region` grown to at least 16 sites of `level` across around its centre, along x and z. */
fn around(region: &Box3, level: u32) -> Box3 {
    let grow = |i: usize| if i == 1 { 0 } else { 0.max((8i64 << level) - (region.max[i] - region.min[i]) / 2) };
    Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) }
}

/**
 * Levels FAR_FROM to FAR see the fields of far regions from above
 * (sites 2 to 8 m apart), against level 0, within FAR_TOLERANCE.
 */
const FAR_FROM: u32 = 4;
const FAR: u32 = 6;

/**
 * How far the fields of a far region may stray from level 0 seen from
 * above. A field is drawn from position alone, and a wide region has
 * hundreds of lines of sites at level 6, so it keeps closer than
 * COLOUR_TOLERANCE, which allows for things' far forms and their
 * surfaces; a plan finer than the sites read where each line is aliases
 * ("Plans" in "Levels" in docs/generators.md).
 */
const FAR_TOLERANCE: f64 = 0.025;

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

/** Each ray's path through `grid` (at `level`), till it leaves its area. */
fn paths(grid: &VoxelGrid, level: u32, rays: &[Ray]) -> Vec<Path> {
    let s = (1i64 << level) as f64;
    let path = |ray: &Ray| composited(grid, ray.origin.map(|v| v / s), ray.direction, ray.end / s);
    rays.iter().map(|ray| path(ray).into_iter().map(|(t, id, stopped, glow)| (t * s, id, stopped, glow)).collect()).collect()
}

/** How much of a ray gets past `t` along `path` (lattice units). */
fn through_to(path: &Path, t: f64) -> f64 {
    1.0 - path.iter().take_while(|(at, ..)| *at <= t).map(|(_, _, stopped, _)| stopped).sum::<f64>()
}

/** Each material id's colour, linear. */
fn linear_colours(generator: &Generator) -> Vec<[f64; 3]> {
    materials_of(generator)
        .iter()
        .map(|m| [16, 8, 0].map(|shift| srgb_to_linear(((m.rgb() >> shift) & 255) as f32 / 255.0) as f64))
        .collect()
}

/**
 * What rays see, as under a white light all round, sRGB: of each site
 * its colour (`linear_colours`) by how much of them it stops, and what it
 * glows to them; for as much light as stops, so that what gets through
 * does not count (Through does).
 */
fn colour(seen: &Seen, colours: &[[f64; 3]]) -> [f64; 3] {
    let (mut sum, mut total) = (seen.glow, 0.0);
    for (&id, &stopped) in seen.stopped.iter().filter(|(id, _)| **id != 0) {
        (0..3).for_each(|c| sum[c] += stopped * colours[id as usize][c]);
        total += stopped;
    }
    sum.map(|v| if total > 0.0 { srgb(v / total) } else { 0.0 })
}

/** Of what stops rays, the share of each material id. */
fn materials(seen: &Seen) -> BTreeMap<u8, f64> {
    shares(seen.stopped.iter().filter(|(id, _)| **id != 0).map(|(&id, &stopped)| (id, stopped)))
}

/** A linear channel, sRGB. */
fn srgb(linear: f64) -> f64 {
    linear_to_srgb(linear as f32) as f64
}

/** How far apart two colours are: in the channel they differ the most in. */
fn colour_difference(a: [f64; 3], c: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - c[i]).abs()).fold(0.0, f64::max)
}

/** Shares of the keys in the total of their weights. */
fn shares(weights: impl IntoIterator<Item = (u8, f64)>) -> BTreeMap<u8, f64> {
    let mut shares = BTreeMap::new();
    let mut total = 0.0;
    for (key, weight) in weights {
        *shares.entry(key).or_insert(0.0) += weight;
        total += weight;
    }
    shares.values_mut().for_each(|v| *v /= total);
    shares
}

/** What a margin measures. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    /** Of what of the rays gets 2 sites of the level far, the share that gets 4 more. */
    Through,
    /** What they see past those 2 sites (`colour`). */
    Colour,
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
    /** How far apart: in the share through, or in a channel of the colours (sRGB). */
    pub difference: f64,
    /** What is measured, at `against` and at `level`: the share through (in the first channel), or a colour (sRGB). */
    pub values: [[f64; 3]; 2],
    /** Of what stops rays, the share of each material id, at `against` and at `level`: to tell what changed, not checked. */
    pub materials: [BTreeMap<u8, f64>; 2],
}

impl Margin {
    pub fn broken(&self) -> bool {
        self.difference >= self.tolerance()
    }

    /** How far it may be apart. */
    pub fn tolerance(&self) -> f64 {
        match self.measure {
            Measure::Through => THROUGH_TOLERANCE,
            Measure::FieldsFromAbove => FAR_TOLERANCE,
            _ => COLOUR_TOLERANCE,
        }
    }
}

/** Shares by kind: its type, its index in Generator::kinds and its name. */
pub type KindShares = BTreeMap<(u8, usize, &'static str), f64>;

/**
 * A world's margins in its regions, to print: `{}` breaks down the broken
 * ones by material type and kind, `{:#}` all of them.
 */
#[derive(Clone)]
pub struct Report {
    pub name: String,
    pub margins: Vec<Margin>,
    /** What regions hold, to print after them: the strata `check` made them for. */
    pub held: Vec<(Box3, String)>,
    /** Each material id's kind (index in Generator::kinds), its name and type; none for id 0. */
    materials: Vec<Option<(usize, &'static str, u8)>>,
}

impl Report {
    fn new(name: &str, generator: &Generator) -> Report {
        // Kinds have consecutive material ids, one per colour, after 0 (none): see materials_of.
        let mut materials = vec![None];
        for (i, kind) in generator.kinds.iter().enumerate() {
            materials.extend(kind.colors.iter().map(|_| Some((i, kind.name, kind.kind_type))));
        }
        Report { name: name.to_string(), margins: Vec::new(), held: Vec::new(), materials }
    }

    /** Only the broken margins. */
    pub fn broken(&self) -> Report {
        Report { margins: self.margins.iter().filter(|m| m.broken()).cloned().collect(), ..self.clone() }
    }

    /** Shares by material type, and by kind, from shares by material id. */
    pub fn by_type_and_kind(&self, shares: &BTreeMap<u8, f64>) -> (BTreeMap<u8, f64>, KindShares) {
        let (mut types, mut kinds) = (BTreeMap::new(), BTreeMap::new());
        for (&id, &share) in shares {
            let Some(Some((kind, name, kind_type))) = self.materials.get(id as usize) else { continue };
            *types.entry(*kind_type).or_insert(0.0) += share;
            *kinds.entry((*kind_type, *kind, *name)).or_insert(0.0) += share;
        }
        (types, kinds)
    }

    /** The shares of a margin's materials by type, then within each by kind, largest differences first. */
    fn breakdown(&self, f: &mut std::fmt::Formatter, m: &Margin) -> std::fmt::Result {
        let [(ta, ka), (tc, kc)] = [&m.materials[0], &m.materials[1]].map(|s| self.by_type_and_kind(s));
        let get = |map: &BTreeMap<u8, f64>, t: u8| map.get(&t).copied().unwrap_or(0.0);
        let mut types: Vec<u8> = ta.keys().chain(tc.keys()).copied().collect::<BTreeSet<_>>().into_iter().collect();
        types.sort_by(|a, b| (get(&tc, *b) - get(&ta, *b)).abs().total_cmp(&(get(&tc, *a) - get(&ta, *a)).abs()));
        for t in types {
            let (a, c) = (get(&ta, t), get(&tc, t));
            if a.max(c) < 0.005 {
                continue;
            }
            let mut kinds: Vec<_> = ka.keys().chain(kc.keys()).filter(|k| k.0 == t).copied().collect::<BTreeSet<_>>().into_iter().collect();
            let kget = |map: &KindShares, k| map.get(&k).copied().unwrap_or(0.0);
            kinds.sort_by(|x, y| (kget(&kc, *y) - kget(&ka, *y)).abs().total_cmp(&(kget(&kc, *x) - kget(&ka, *x)).abs()));
            let kinds: Vec<String> = kinds
                .iter()
                .filter(|k| kget(&ka, **k).max(kget(&kc, **k)) >= 0.005)
                .map(|k| format!("{} {:.3} -> {:.3}", k.2, kget(&ka, *k), kget(&kc, *k)))
                .collect();
            writeln!(f, "      type {t:>3}: {a:.3} -> {c:.3} ({:+.3}): {}", c - a, kinds.join(", "))?;
        }
        Ok(())
    }
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
                Measure::Through => "rays through",
                Measure::Colour => "colour",
                Measure::FromAbove => "colour from above",
                Measure::FieldsFromAbove => "fields' colour from above",
            };
            let [a, c] = m.values;
            let values = match m.measure {
                Measure::Through => format!("{:.3} -> {:.3}", a[0], c[0]),
                _ => format!("{:.3} {:.3} {:.3} -> {:.3} {:.3} {:.3}", a[0], a[1], a[2], c[0], c[1], c[2]),
            };
            let broken = if m.broken() { format!(", breaks {}", m.tolerance()) } else { String::new() };
            writeln!(f, "  level {} against {}, {what}: {values}: {:.3} apart{broken}", m.level, m.against, m.difference)?;
            if !m.materials[0].is_empty() && (m.broken() || f.alternate()) {
                self.breakdown(f, m)?;
            }
        }
        Ok(())
    }
}

/**
 * What rays from where a camera could be see of `region` at each level
 * from 1 to `levels`: as much gets through, and of what stops, as much
 * that they see of the same colour, as at the level two finer, within
 * THROUGH_TOLERANCE and COLOUR_TOLERANCE.
 */
fn levels_against_finer(generator: &Generator, region: &Box3, levels: u32, report: &mut Report) {
    let colours = linear_colours(generator);
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
        let fine = paths(&grids[reference].2, finer, rays);
        let coarse_grid = grid(&mut grids, level, &area);
        let coarse = paths(&grids[coarse_grid].2, level, rays);
        // A level is seen from afar: of the light of rays that got 2 of its sites far, how much gets 4 more,
        // and what stops the rest. Not counted: what leaves the area before it could get the 4 more.
        let (from, d) = (2.0 * s, 4.0 * s);
        let through = |paths: &[Path]| {
            let (mut seen, mut through) = (0.0, 0.0);
            for (path, ray) in paths.iter().zip(rays) {
                if ray.end > from + d {
                    seen += through_to(path, from);
                    through += through_to(path, from + d);
                } else if ray.end > from {
                    seen += through_to(path, from) - through_to(path, ray.end);
                }
            }
            through / seen
        };
        let seen = |paths: &[Path]| {
            let mut seen = Seen::default();
            paths.iter().flatten().filter(|(t, ..)| *t > from).for_each(|&(_, id, stopped, glow)| seen.add(id, stopped, glow));
            seen
        };
        let margin = |measure, values: [[f64; 3]; 2], materials| Margin {
            region: *region,
            measure,
            level,
            against: finer,
            difference: if measure == Measure::Through {
                (values[1][0] - values[0][0]).abs()
            } else {
                colour_difference(values[0], values[1])
            },
            values,
            materials,
        };
        report.margins.push(margin(Measure::Through, [through(&fine), through(&coarse)].map(|t| [t, 0.0, 0.0]), Default::default()));
        let seen = [seen(&fine), seen(&coarse)];
        report.margins.push(margin(Measure::Colour, seen.each_ref().map(|s| colour(s, &colours)), seen.each_ref().map(materials)));
        // Only the grids the coming levels read: the coarse one, and the one after it.
        grids.retain(|(l, ..)| *l + 1 >= level);
    }
}

/**
 * What a camera straight above `region`, grown to at least 16 sites of
 * the level across around its centre, sees at each level from 1 to
 * `levels`: what rays down see (`colour`), as at level 0, within
 * COLOUR_TOLERANCE. Unlike levels two apart, level 0 catches a drift,
 * a little a level, such as walls taking over from roofs.
 */
fn from_above_against_level_0(world: &World, region: &Box3, levels: u32, report: &mut Report) {
    from_above(world, region, 1..=levels, Measure::FromAbove, report);
}

/** What rays straight down over `region` see at each of `levels`, against level 0, as `measure`. */
fn from_above(world: &World, region: &Box3, levels: impl Iterator<Item = u32>, measure: Measure, report: &mut Report) {
    let colours = linear_colours(&world.generator);
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
        let values = [colour(reference, &colours), colour(&here, &colours)];
        let difference = colour_difference(values[0], values[1]);
        let materials = [materials(reference), materials(&here)];
        report.margins.push(Margin { region: *region, measure, level, against: 0, difference, values, materials });
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
 * ones (only x and z count), at levels FAR_FROM to FAR,
 * against level 0. Its things are left out, and with them their far
 * fields: their far forms keep what rays see around them, not from
 * above ("Media are isotropic" in docs/generators.md).
 */
pub fn far_margins(name: &str, world: World, regions: &[Box3]) -> Report {
    let world = fields_of(world);
    let mut report = Report::new(name, &world.generator);
    for region in regions {
        from_above(&world, region, FAR_FROM..=FAR, Measure::FieldsFromAbove, &mut report);
    }
    report
}

/** `far_margins`, printed; panics where one breaks FAR_TOLERANCE. */
pub fn check_far(name: &str, world: World, regions: &[Box3]) {
    let start = Instant::now();
    let report = far_margins(name, world, regions);
    println!("{report}{name}: far {:.1} s", start.elapsed().as_secs_f64());
    let broken = report.broken();
    assert!(broken.margins.is_empty(), "{name} breaks the contract far away:\n{broken}");
}

/** Sites of each level above and below a world's heights that `heights_hold` generates. */
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
 * outside them: at each level up to one past `levels`, in each region
 * and a host tile's width around spawn, BEYOND sites above and below
 * the heights are empty. Returns where they are not.
 */
pub fn heights_hold(world: &World, regions: &[Box3], levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    for level in 0..=levels + 1 {
        let (low, high) = world.heights_at(level);
        // A host's tile: 128 sites wide.
        let half = 64 << level;
        let [x, _, z] = world.spawn;
        let spawn = Box3 { min: [x - half, 0, z - half], max: [x + half, 0, z + half] };
        for (i, area) in regions.iter().chain([&spawn]).enumerate() {
            let b = box_at(area, level);
            for (from, to, side) in [(high, high + BEYOND, "above"), (low - BEYOND, low, "below")] {
                let slab = Box3 { min: [b.min[0], from, b.min[2]], max: [b.max[0], to, b.max[2]] };
                if let Some((at, kind, n)) = stopping(&world.generator, &generate(&world.generator, &slab, level), level) {
                    let place = if i == regions.len() { "around spawn".to_string() } else { format!("in {area:?}") };
                    broken.push(format!(
                        "level {level}, {place}: {n} sites stopping light {side} the heights {:?} (World::heights), which hosts don't generate, \
                         such as {kind} at {at:?} lattice units",
                        world.heights
                    ));
                }
            }
        }
    }
    broken
}

/** Sites a side of the boxes `fields_hold` asks fields about: small, so they are asked close to what they draw. */
const ASKED: i64 = 32;

/**
 * Each field draws nothing in a box it says it may not draw in
 * (Field::within): in each region, at the levels `whole_equals_parts`
 * checks, cut into boxes ASKED sites a side, those it says so of are
 * drawn all the same. Returns where they drew.
 */
pub fn fields_hold(world: &World, regions: &[Box3], levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    for level in [0, 2, levels + 1] {
        for region in regions {
            let b = box_at(region, level);
            let starts = |a: usize| (b.min[a]..b.max[a]).step_by(ASKED as usize);
            for z in starts(2) {
                for y in starts(1) {
                    for x in starts(0) {
                        let part = Box3 { min: [x, y, z], max: [x + ASKED, y + ASKED, z + ASKED] }.intersect(&b);
                        let (_, drew) = checking_within(|| generate(&world.generator, &part, level));
                        broken.extend(drew.into_iter().map(|d| format!("level {level}: {d}")));
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
 * holds places of strata (worldgen::survey), as many as fit.
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

/** What `check` found: where `world` breaks the contract. */
pub struct Checked {
    pub name: String,
    /** Each region, the strata it holds, and how it breaks the contract there (its margins), if it does. */
    pub regions: Vec<(Box3, Vec<String>, String)>,
    /** How it breaks it elsewhere: its heights, things' reach. */
    pub broken: Vec<String>,
    /** The layers of its generator that draw nothing surveyed, by name: none checked, if `complete`. */
    pub silent: Vec<&'static str>,
    /** Whether the regions hold every stratum, not `only` some. */
    pub complete: bool,
}

impl Checked {
    /**
     * Panics where it breaks the contract, but in regions holding a
     * stratum whose name holds one of `known`; and where one of `known`
     * no longer breaks it, to be taken off the list.
     */
    pub fn assert_kept(&self, known: &[&str]) {
        let holds = |strata: &[String], k: &str| strata.iter().any(|s| s.contains(k));
        let mut broken = self.broken.clone();
        if self.complete && !self.silent.is_empty() {
            broken.push(format!("{} draw nothing around spawn (worldgen::survey): too far from it, or not at all", self.silent.join(", ")));
        }
        for (_, strata, breaks) in &self.regions {
            if !breaks.is_empty() && !known.iter().any(|k| holds(strata, k)) {
                broken.push(breaks.clone());
            }
        }
        for k in known {
            let held: Vec<_> = self.regions.iter().filter(|(_, strata, _)| holds(strata, k)).collect();
            if (self.complete || !held.is_empty()) && held.iter().all(|(.., breaks)| breaks.is_empty()) {
                broken.push(format!("{} keeps the contract around {k}: take it off the known breaks", self.name));
            }
        }
        assert!(broken.is_empty(), "{} breaks the contract:\n{}", self.name, broken.join("\n"));
    }
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
    broken.extend(fields_hold(world, &boxes, levels));
    let fields = start.elapsed().as_secs_f64() - rays - heights - things;
    println!("{report}{name}: margins {rays:.1} s, heights {heights:.1} s, things {things:.1} s, fields {fields:.1} s");
    let regions = regions
        .into_iter()
        .map(|(b, strata)| {
            let breaks = Report { margins: report.broken().margins.into_iter().filter(|m| m.region == b).collect(), ..report.clone() };
            (b, strata, if breaks.margins.is_empty() { String::new() } else { breaks.to_string() })
        })
        .collect();
    let silent = survey.layers.iter().filter(|l| l.found == 0).map(|l| l.layer).collect();
    Checked { name: name.to_string(), regions, broken, silent, complete: only.is_empty() }
}
