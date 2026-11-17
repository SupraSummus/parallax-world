//! The contract every world keeps (docs/generators.md, "Checks"), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; nothing lies outside its heights, things keep their
//! reach, and what is skipped where it says it may not draw draws nothing
//! there; and on average over each stratum's regions, each level's cells
//! look as what is in them does, the level two finer (checks::cells). What
//! is seen is a `Look`: linear, as a coarse site averages finer ones, so
//! the sum of each kind's, which tell what changed.

use std::collections::BTreeMap;
use std::ops::Range;
use std::time::Instant;

use fcc::FACE_KINDS;

use crate::World;
use crate::checks::cells;
use crate::checks::survey::{Survey, survey};
use crate::generator::{Generator, Layer, Whole, checking_skipped, generate, kind_of, whole};
use crate::grid::{ALPHA, Box3, VoxelGrid, is_site};
use crate::noise::Rng;
use crate::picture::{Camera, Seen, Tiles, composite_in};
use crate::units::m;

fn b(min_x: i64, min_y: i64, min_z: i64, max_x: i64, max_y: i64, max_z: i64) -> Box3 {
    Box3 { min: [min_x, min_y, min_z], max: [max_x, max_y, max_z] }
}

/** The box of `level`'s sites covering `region` and a margin of two sites. */
pub(crate) fn box_at(r: &Box3, level: u32) -> Box3 {
    let s = 1i64 << level;
    Box3 { min: r.min.map(|v| v.div_euclid(s) - 2), max: r.max.map(|v| -(-v).div_euclid(s) + 2) }
}

pub(crate) fn sites(b: &Box3) -> impl Iterator<Item = (i64, i64, i64)> + '_ {
    (b.min[2]..b.max[2]).flat_map(move |z| {
        (b.min[1]..b.max[1]).flat_map(move |y| {
            let from = b.min[0] + ((b.min[0] + y + z) & 1);
            (from..b.max[0]).step_by(2).map(move |x| (x, y, z))
        })
    })
}

/**
 * How far a level's cells may look from what is in them, the level
 * FINER_BY finer (checks::cells), in any channel, unless a world says
 * otherwise: what they add or lose on average over a stratum's regions
 * (`breaks`), or a solid's alone, in a region of many
 * (worldgen/tests/shapes.rs).
 */
pub const TOLERANCE: f64 = 0.045;

/**
 * How far a level's cells may be from what is in them, each in its
 * channel furthest, on average, unless a world says otherwise: a site
 * seen as a whole looks as its cell does to within the steps of what it
 * holds (docs/generators.md, "Levels").
 */
pub const SPREAD: f64 = 0.1;

/** How closely a level's cells look as what is in them, on average: how far apart, and how spread. */
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accuracy {
    pub apart: f64,
    pub spread: f64,
}

impl Accuracy {
    pub const DEFAULT: Accuracy = Accuracy { apart: TOLERANCE, spread: SPREAD };

    /**
     * How closely `stratum` is to keep the contract, of those `declared`
     * by name: the stratum named, or the nearest part of a thing or layer
     * it is in ("towns" for "towns/block/house wall"); DEFAULT for one
     * in none.
     */
    pub fn of(declared: &[(&str, Accuracy)], stratum: &str) -> Accuracy {
        declared.iter().filter(|(name, _)| covers(name, stratum)).max_by_key(|(name, _)| name.len()).map_or(Accuracy::DEFAULT, |(_, a)| *a)
    }
}

/** Whether `stratum` is the one named `name`, or a part (after a '/') or a kind (after a space) of it. */
pub fn covers(name: &str, stratum: &str) -> bool {
    stratum == name || stratum.strip_prefix(name).is_some_and(|rest| rest.starts_with([' ', '/']))
}

/** `region` grown to at least 16 sites of `level` across around its centre, along x and z. */
pub(crate) fn around(region: &Box3, level: u32) -> Box3 {
    let grow = |i: usize| if i == 1 { 0 } else { 0.max((8i64 << level) - (region.max[i] - region.min[i]) / 2) };
    Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) }
}

/**
 * Levels from FAR_FROM see the fields of far regions from above (sites
 * 2 m apart and more), against level 0, within FAR.
 */
const FAR_FROM: u32 = 4;

/**
 * How far the fields of a far region may stray from level 0 seen from
 * above, each region. A field is drawn from position alone, and a wide
 * region has hundreds of lines of sites at level 6, so it keeps close; a
 * plan finer than the sites read where each line is aliases ("Plans" in
 * "Levels" in docs/generators.md).
 */
const FAR: Accuracy = Accuracy { apart: 0.0125, spread: f64::INFINITY };

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
            // What shows through each face and what rays and light see there, and what stops a walking body.
            let look = |g: &VoxelGrid| {
                let faces = std::array::from_fn::<_, FACE_KINDS, _>(|face| (g.face(x, y, z, face), g.light(x, y, z, face)));
                (faces, g.solid(x, y, z))
            };
            assert_eq!(look(&alone), look(&grid), "level {level}: part {part:?} differs at ({x}, {y}, {z})");
        }
    }
}

/**
 * What rays see, a mean over them, linear, as under a white light all
 * round: how much of them stops (`STOPPED`), and what shows (`SHOWN`):
 * what stops each by its colour, and what glows to them, premultiplied as
 * compositing has it. With no ratio, a few rays of many change it by no
 * more than their share; with no sky behind, over any sky it differs by
 * no more than its channels do.
 */
pub type Look = cells::Look;

/** The channel of a `Look` that says how much stops. */
pub const STOPPED: Range<usize> = 0..1;

/** The channels of a `Look` that say what shows: red, green and blue. */
pub const SHOWN: Range<usize> = 1..4;

/** Less than this stopped, a look has no colour to compare. */
const SEEN: f64 = 1e-3;

/**
 * How far apart two looks are in `channels`, in the one they differ the
 * most in: how much stops, and the colour of what stops (what shows over
 * how much stops), so a thin medium's colour or glow counts as much as a
 * solid's.
 */
pub fn apart(a: &Look, c: &Look, channels: Range<usize>) -> f64 {
    let colour = |l: &Look, i: usize| if i == 0 { l[0] } else { l[i] / l[0] };
    let coloured = a[0] >= SEEN && c[0] >= SEEN;
    channels.filter(|&i| i == 0 || coloured).map(|i| (colour(c, i) - colour(a, i)).abs()).fold(0.0, f64::max)
}

/** The look of what pixels see (a mean over them). */
fn look_of(seen: &Seen) -> Look {
    [seen.stopped, seen.shown[0] + seen.glow[0], seen.shown[1] + seen.glow[1], seen.shown[2] + seen.glow[2]]
}

/** What a margin measures. */
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Measure {
    /** What each of a sample of cells looks like along each of the lattice's directions, against what is in it (checks::cells). */
    Cells,
    /** What rays straight down see of the fields alone, against level 0. */
    FieldsFromAbove,
}

/** How far a level of a world strays from another in a region, by one measure. */
#[derive(Clone, Debug)]
pub struct Margin {
    pub region: Box3,
    pub measure: Measure,
    pub level: u32,
    /** The level it is measured against: FINER_BY finer, or 0 from above. */
    pub against: u32,
    /** What is seen at `against` and at `level`, on average. */
    pub looks: [Look; 2],
    /** How far, on average, each of its cells is from what is in it, in its channel furthest; none from above. */
    pub spread: f64,
}

impl Margin {
    /** How far apart the looks are in `channels` (`apart`). */
    pub fn apart(&self, channels: Range<usize>) -> f64 {
        apart(&self.looks[0], &self.looks[1], channels)
    }

    /** How far apart the looks are, in any channel. */
    pub fn difference(&self) -> f64 {
        self.apart(0..4)
    }

    /** Whether this region alone is as far apart, or as spread, as a stratum may be on average `within`, or more. */
    pub fn broken(&self, within: Accuracy) -> bool {
        self.difference() >= within.apart || self.spread >= within.spread
    }
}

/** A world's margins in its regions, to print. */
#[derive(Clone)]
pub struct Report {
    pub name: String,
    pub margins: Vec<Margin>,
    /** What regions hold, to print after them: the strata `check` made them for. */
    pub held: Vec<(Box3, String)>,
}

impl Report {
    fn new(name: &str) -> Report {
        Report { name: name.to_string(), margins: Vec::new(), held: Vec::new() }
    }

    /** Only the margins broken `within` (`Margin::broken`). */
    pub fn broken(&self, within: Accuracy) -> Report {
        Report { margins: self.margins.iter().filter(|m| m.broken(within)).cloned().collect(), ..self.clone() }
    }
}

/** A look, to print: how much stops, and what shows. */
fn look(l: &Look) -> String {
    format!("stops {:.3}, shows {:.3} {:.3} {:.3}", l[0], l[1], l[2], l[3])
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
                Measure::Cells => format!("cells, spread {:.3}", m.spread),
                Measure::FieldsFromAbove => "fields from above".to_string(),
            };
            let [a, c] = m.looks.map(|v| look(&v));
            writeln!(f, "  level {} against {}, {what}: {a} -> {c}: {:.3} apart", m.level, m.against, m.difference())?;
        }
        Ok(())
    }
}

/**
 * What rays straight down over `region` see at each of `levels`, against
 * level 0, as `Measure::FieldsFromAbove`: level 0's tiles in `level_0`, which keeps those
 * this region shares with the one before.
 */
fn from_above(world: &World, region: &Box3, levels: impl Iterator<Item = u32>, level_0: &mut Tiles, report: &mut Report) {
    let levels: Vec<u32> = levels.collect();
    if world.heights.is_none() {
        assert!(levels.is_empty(), "a world with no heights has no sky to be seen from (World::heights)");
        return;
    }
    let seen = |tiles: &mut Tiles, area: &Box3| {
        // A ray every 2 units, through a random point of each 2 × 2: through their middles, rays meet plans of whole
        // units at one phase, and the lattice's vertices, where a ray goes on into a voxel by its side.
        let width = ((area.max[0] - area.min[0]) / 2).clamp(16, 1024) as usize;
        let camera = Camera::top([area.min[0] as f64, area.min[2] as f64], [area.max[0] as f64, area.max[2] as f64], width);
        let camera = camera.jittered(13);
        composite_in(&world.generator, &camera, tiles)
    };
    // Level 0 over the area of the levels since it last grew; the areas
    // nest, so its tiles are kept for the next. Rays come down from the
    // world's sky, over its heights, cutting nothing.
    if let Some(&widest) = levels.last() {
        level_0.keep(&around(region, widest));
    }
    let mut reference: Option<(Box3, Seen)> = None;
    for level in levels {
        let area = around(region, level);
        if reference.as_ref().is_none_or(|(a, _)| *a != area) {
            reference = Some((area, seen(level_0, &area)));
        }
        let reference = &reference.as_ref().expect("made").1;
        let here = seen(&mut Tiles::new(level, world.heights_at(level)), &area);
        let looks = [look_of(reference), look_of(&here)];
        report.margins.push(Margin { region: *region, measure: Measure::FieldsFromAbove, level, against: 0, looks, spread: 0.0 });
    }
}

/**
 * How each level of `world`, named `name` (with its seed, say), up to
 * `levels` looks in each of `regions` against what is in its cells, the
 * level FINER_BY finer (checks::cells): its cells near a surface, a
 * sample of them, each along each of the lattice's 12 directions.
 */
pub fn margins(name: &str, world: &World, regions: &[Box3], levels: u32) -> Report {
    margins_near(name, world, None, regions, levels)
}

/** `margins`, of the cells near a surface of `focus` too, if given. */
fn margins_near(name: &str, world: &World, focus: Option<&Generator>, regions: &[Box3], levels: u32) -> Report {
    let mut report = Report::new(name);
    for region in regions {
        for level in 1..=levels {
            let c = cells::cells_of(name, world, focus, region, level);
            let n = c.cells.len().max(1) as f64;
            let looks = [0, 1].map(|i| std::array::from_fn(|k| c.cells.iter().map(|cell| cell.looks[i][k]).sum::<f64>() / n));
            let spread = c.cells.iter().map(cells::Cell::deviation).sum::<f64>() / n;
            report.margins.push(Margin { region: *region, measure: Measure::Cells, level, against: c.against, looks, spread });
        }
    }
    report
}

/**
 * What `world`'s fields show from above over each of `regions`, wide
 * ones, at levels FAR_FROM to `levels`, against
 * level 0. Its things are left out, and with them their far
 * fields: their far forms keep what rays see around them, not from
 * above ("Media are isotropic" in docs/generators.md).
 */
pub fn far_margins(name: &str, world: World, regions: &[Box3], levels: u32) -> Report {
    let world = fields_of(world);
    let mut report = Report::new(name);
    let mut level_0 = Tiles::new(0, world.heights_at(0));
    for region in regions {
        from_above(&world, region, FAR_FROM..=levels, &mut level_0, &mut report);
    }
    report
}

/** `far_margins`, printed; panics where one breaks FAR. */
pub fn check_far(name: &str, world: World, regions: &[Box3], levels: u32) {
    let start = Instant::now();
    let report = far_margins(name, world, regions, levels);
    println!("{report}{name}: far {:.1} s", start.elapsed().as_secs_f64());
    let broken = report.broken(FAR);
    assert!(broken.margins.is_empty(), "{name} breaks the contract far away, over {}:\n{broken}", FAR.apart);
}

/** Sites of each level above a world's heights that `heights_hold` generates. */
const BEYOND: i64 = 16;

/** Where a level `level` box of `grid` has a site that stops any light (its alpha), in lattice units, and its kind; how many it has. */
fn stopping(generator: &Generator, grid: VoxelGrid, level: u32) -> Option<([i64; 3], &'static str, usize)> {
    let b = grid.clip();
    let full: Vec<_> = sites(&b).filter(|&(x, y, z)| grid.alpha(x, y, z) != 0).collect();
    let &(x, y, z) = full.first()?;
    let kind = kind_of(generator)(grid.inside(x, y, z)).map_or("a mix", |k| k.name);
    Some(([x, y, z].map(|v| v << level), kind, full.len()))
}

/**
 * Hosts generate a world only within its heights, so nothing may lie
 * over them: at each level up to one past `levels`, in each region and a
 * host tile's width around spawn, BEYOND sites above the heights are
 * empty. Under them the ground may go on (Field::ground), unseen. Returns
 * where they are not.
 */
pub fn heights_hold(world: &World, regions: &[Box3], levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    if world.heights.is_none() {
        return broken;
    }
    for level in 0..=levels + 1 {
        let high = world.heights_at(level).1;
        // A host's tile: 128 sites wide.
        let half = 64 << level;
        let [x, _, z] = world.spawn;
        let spawn = Box3 { min: [x - half, 0, z - half], max: [x + half, 0, z + half] };
        for (i, area) in regions.iter().chain([&spawn]).enumerate() {
            let b = box_at(area, level);
            let slab = Box3 { min: [b.min[0], high, b.min[2]], max: [b.max[0], high + BEYOND, b.max[2]] };
            if let Some((at, kind, n)) = stopping(&world.generator, generate(&world.generator, &slab, level), level) {
                let place = if i == regions.len() { "around spawn".to_string() } else { format!("in {area:?}") };
                broken.push(format!(
                    "level {level}, {place}: {n} sites stopping light above the heights {:?} (World::heights), which hosts don't generate, \
                     such as {kind} at {at:?} lattice units",
                    world.heights.expect("bounded")
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
                            Whole::Buried(_) => sites.into_iter().find(|&[x, y, z]| grid.alpha(x, y, z) < ALPHA),
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
    for things in &survey.things {
        for (key, bounds) in &things.leaving {
            let (h0, h1) = world.heights.expect("the survey keeps things leaving a world's heights, if it has them");
            for (from, to, side) in [(h1, bounds.max[1] + 2, "above"), (bounds.min[1] - 2, h0, "below")] {
                if from >= to {
                    continue;
                }
                let b = Box3 { min: [bounds.min[0] - 2, from, bounds.min[2] - 2], max: [bounds.max[0] + 2, to, bounds.max[2] + 2] };
                if let Some((at, kind, n)) = stopping(&world.generator, generate(&world.generator, &b, 0), 0) {
                    broken.push(format!(
                        "{key}: {n} sites stopping light {side} the heights {:?} (World::heights), which hosts don't generate, \
                         such as {kind} at {at:?} lattice units",
                        (h0, h1)
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
 * names hold one of `only`, if any (a layer's name first:
 * "towns/church", "railway"), and only their layers surveyed.
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
 * Where the contract breaks: the stratum and level of a mean that breaks
 * it, if one does, with how far apart and how spread it is; and what
 * breaks.
 */
pub struct Break {
    pub of: Option<(String, u32, f64, f64)>,
    pub message: String,
}

/** How far a baseline's mean may move either way before it is to be written anew. */
const SLACK: f64 = 0.01;

/**
 * Panics where `breaks` differ from the baseline at `path`, a line per
 * stratum and level that breaks the contract, how far apart and how
 * spread: what breaks it and isn't there, or is further apart or more
 * spread by more than SLACK; and, if `all` were checked, what is there
 * and now nearer or kept, by as much. CONTRACT_BLESS=1 (with `all`)
 * writes the baseline anew instead.
 */
pub fn hold_to_baseline(breaks: &[Break], path: &str, all: bool) {
    if all && std::env::var_os("CONTRACT_BLESS").is_some() {
        let mut lines: Vec<String> = breaks
            .iter()
            .filter_map(|b| b.of.as_ref())
            .map(|(s, l, apart, spread)| format!("{s}\t{l}\t{apart:.3}\t{spread:.3}\n"))
            .collect();
        lines.sort();
        std::fs::write(path, lines.concat()).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    let baseline = std::fs::read_to_string(path).unwrap_or_default();
    let listed: Vec<(&str, u32, f64, f64)> = baseline
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let number = |i: usize| f[i].parse::<f64>().unwrap_or_else(|_| panic!("{path}: {l}"));
            (f[0], number(1) as u32, number(2), number(3))
        })
        .collect();
    let mut changed = Vec::new();
    for b in breaks {
        let Some((stratum, level, apart, spread)) = &b.of else {
            changed.push(b.message.clone());
            continue;
        };
        match listed.iter().find(|l| l.0 == stratum && l.1 == *level) {
            None => changed.push(b.message.clone()),
            Some(&(_, _, was, was_spread)) if *apart > was + SLACK || *spread > was_spread + SLACK => {
                changed.push(format!("worse than its baseline ({was:.3} apart, spread {was_spread:.3}): {}", b.message))
            }
            Some(&(_, _, was, was_spread)) if all && (*apart < was - SLACK || *spread < was_spread - SLACK) => changed.push(format!(
                "{stratum}, level {level}: better than its baseline ({was:.3} apart, spread {was_spread:.3}): {apart:.3}, {spread:.3}"
            )),
            Some(_) => {}
        }
    }
    if all {
        for (stratum, level, _, _) in &listed {
            if !breaks.iter().any(|b| b.of.as_ref().is_some_and(|(s, l, _, _)| s == stratum && l == level)) {
                changed.push(format!("{stratum}, level {level}: in its baseline, but keeps the contract now"));
            }
        }
    }
    assert!(changed.is_empty(), "{}\nif that is meant, CONTRACT_BLESS=1 writes {path} anew", changed.join("\n"));
}

/**
 * Where worlds checked (at several seeds, say) break the contract: the
 * rules that hold exactly, layers that draw nothing, and each stratum's
 * margins (by measure and level) averaged over the regions holding it
 * at every seed, if they are as far apart or as spread as it is to keep
 * it (as `declared` by name, `Accuracy::of`), or more; and, if every
 * stratum was checked, a name declared that covers none.
 * Each broken mean names the regions it was averaged over, and the
 * worst of them broken down by kind. Each with what it is of.
 */
pub fn breaks(checked: &[Checked], declared: &[(&str, Accuracy)]) -> Vec<Break> {
    let mut broken = Vec::new();
    let mut push = |of: Option<(String, u32, f64, f64)>, message: String| broken.push(Break { of, message });
    for c in checked {
        c.broken.iter().for_each(|b| push(None, format!("{}: {b}", c.name)));
        if c.complete && !c.silent.is_empty() {
            push(
                None,
                format!(
                    "{}: {} draw nothing around spawn or the places it names (worldgen::checks::survey): too far from them, or not at all",
                    c.name,
                    c.silent.join(", ")
                ),
            );
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
    if checked.iter().all(|c| c.complete) {
        for (name, _) in declared.iter().filter(|(name, _)| !means.keys().any(|(s, ..)| covers(name, s))) {
            push(None, format!("an accuracy is declared for {name}, which no stratum surveyed is or is in"));
        }
    }
    for ((stratum, measure, level), margins) in means {
        let n = margins.len() as f64;
        let mean = |i: usize| -> Look { std::array::from_fn(|k| margins.iter().map(|(_, m)| m.looks[i][k]).sum::<f64>() / n) };
        let means = [mean(0), mean(1)];
        let apart = apart(&means[0], &means[1], 0..4);
        let mean: Look = std::array::from_fn(|k| means[1][k] - means[0][k]);
        let spread = margins.iter().map(|(_, m)| m.spread).sum::<f64>() / n;
        let within = Accuracy::of(declared, stratum);
        if apart < within.apart && spread < within.spread {
            continue;
        }
        let mut message = format!(
            "{stratum}, level {level}, {measure:?}: {apart:.3} apart, spread {spread:.3} (within {}, {}), on average over {} regions (stops {:+.3}, shows {:+.3} {:+.3} {:+.3}):",
            within.apart,
            within.spread,
            margins.len(),
            mean[0],
            mean[1],
            mean[2],
            mean[3]
        );
        for (c, m) in &margins {
            message += &format!("\n  {} in {:?}: {:.3}", c.name, m.region, m.difference());
        }
        let (c, worst) = margins.iter().max_by(|a, b| a.1.difference().total_cmp(&b.1.difference())).expect("a margin");
        message += &format!("\n{:#}", Report { margins: vec![(*worst).clone()], ..c.report.clone() });
        push(Some((stratum.to_string(), level, apart, spread)), message);
    }
    broken
}

/**
 * Each of `regions` of `world` generated in parts equals it generated
 * whole (`whole_equals_parts`), up to one level past `levels`: where
 * things may be drawn as media (a forest's trees). Prints how long it
 * took, as `name`.
 */
pub fn parts_equal_whole(name: &str, world: &World, regions: &[Box3], levels: u32) {
    let start = Instant::now();
    for region in regions {
        for level in [0, 2, levels + 1] {
            whole_equals_parts(&world.generator, region, level);
        }
    }
    println!("{name}: parts {:.1} s", start.elapsed().as_secs_f64());
}

/**
 * The whole contract for `world`, named `name` (with its seed, say), in
 * regions holding everything it is made of around spawn (`regions`),
 * `only` some if any, up to the level `levels` its cells look as what
 * is in them to. Hands `surveyed`
 * the regions once found: to check them in parts (`parts_equal_whole`)
 * meanwhile, on a thread of their own, say. Prints its regions and
 * margins (see them with --nocapture); returns where it breaks it.
 */
pub fn check(name: &str, world: &World, levels: u32, only: &[&str], surveyed: impl FnOnce(&[Box3])) -> Checked {
    let start = Instant::now();
    let (survey, regions) = regions(world, only);
    print_survey(name, &survey, regions.len(), start);
    if only.is_empty() {
        // Not checked: a world may share a table of kinds with others (the moor's is the valley's).
        println!("{name}: kinds nothing surveyed draws: {}", survey.unseen(&world.generator).join(", "));
    }
    let boxes: Vec<Box3> = regions.iter().map(|(b, _)| *b).collect();
    surveyed(&boxes);
    let mut broken = Vec::new();
    let start = Instant::now();
    let report =
        Report { held: regions.iter().map(|(b, strata)| (*b, strata.join(", "))).collect(), ..margins(name, world, &boxes, levels) };
    let cells = start.elapsed().as_secs_f64();
    broken.extend(heights_hold(world, &boxes, levels));
    let heights = start.elapsed().as_secs_f64() - cells;
    broken.extend(things_hold(world, &survey));
    let things = start.elapsed().as_secs_f64() - cells - heights;
    broken.extend(skips_hold(world, &boxes, levels));
    let skips = start.elapsed().as_secs_f64() - cells - heights - things;
    println!("{report}{name}: cells {cells:.1} s, heights {heights:.1} s, things {things:.1} s, skips {skips:.1} s");
    let silent = survey.layers.iter().filter(|l| l.found == 0).map(|l| l.layer).collect();
    Checked { name: name.to_string(), regions, report, broken, silent, complete: only.is_empty() }
}

/**
 * The contract for a `component` of the world `make` makes (`World::alone`),
 * at its `places` places nearest spawn as the survey finds it, none over
 * another, a thread each, up to `levels`: its cells near its surfaces and
 * the world's, all around it drawn as the world draws it, and each place
 * in parts equals it whole. Its margins are the component's, by name.
 * Alone if `make` makes the world cut to it.
 */
pub fn check_component(name: &str, make: &(dyn Fn() -> World + Sync), component: &str, places: usize, levels: u32) -> Checked {
    let start = Instant::now();
    let focus = make().alone(component);
    let survey = survey(&focus, REGION, |_| true);
    let boxes = nearest(&survey, places);
    print_survey(name, &survey, boxes.len(), start);
    let start = Instant::now();
    let each: Vec<(Vec<Margin>, Vec<String>)> = std::thread::scope(|scope| {
        let threads: Vec<_> = boxes
            .iter()
            .map(|b| {
                // Worlds aren't Send: each thread makes its own.
                scope.spawn(move || {
                    let (world, focus) = (make(), make().alone(component));
                    let margins = margins_near(name, &world, Some(&focus.generator), &[*b], levels).margins;
                    [0, 2, levels + 1].into_iter().for_each(|level| whole_equals_parts(&world.generator, b, level));
                    let mut broken = heights_hold(&world, &[*b], levels);
                    broken.extend(skips_hold(&world, &[*b], levels));
                    (margins, broken)
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    });
    let (margins, broken): (Vec<_>, Vec<_>) = each.into_iter().unzip();
    let mut broken: Vec<String> = broken.into_iter().flatten().chain(things_hold(&focus, &survey)).collect();
    broken.sort();
    broken.dedup();
    let regions: Vec<(Box3, Vec<String>)> = boxes.iter().map(|b| (*b, vec![component.to_string()])).collect();
    let held = boxes.iter().map(|b| (*b, component.to_string())).collect();
    let report = Report { name: name.to_string(), margins: margins.into_iter().flatten().collect(), held };
    println!("{report}{name}: checked in {:.1} s", start.elapsed().as_secs_f64());
    let silent = survey.layers.iter().filter(|l| l.found == 0).map(|l| l.layer).collect();
    Checked { name: name.to_string(), regions, report, broken, silent, complete: true }
}

/** Of the places of `survey`'s strata, the `n` nearest its spawn (along x and z), none overlapping one nearer. */
fn nearest(survey: &Survey, n: usize) -> Vec<Box3> {
    let mut places: Vec<Box3> = survey.strata.iter().flat_map(|s| s.places.iter().copied()).collect();
    let distance = |b: &Box3| [0, 2].map(|a| ((b.min[a] + b.max[a]) / 2 - survey.spawn[a]).pow(2)).iter().sum::<i64>();
    places.sort_by_key(|b| (distance(b), b.min, b.max));
    let mut nearest: Vec<Box3> = Vec::new();
    for place in places {
        if nearest.len() < n && !nearest.iter().any(|b| b.overlaps(&place)) {
            nearest.push(place);
        }
    }
    nearest
}

/** Prints what `survey` found, and in how long since `start`. */
fn print_survey(name: &str, survey: &Survey, regions: usize, start: Instant) {
    let layers: Vec<String> = survey
        .layers
        .iter()
        .map(|l| format!("{} {:.1} km {:.1} s", l.layer, 2.0 * l.half as f64 * crate::units::UNIT_SIZE / 1000.0, l.seconds))
        .collect();
    let (n, took) = (survey.strata.len(), start.elapsed().as_secs_f64());
    println!("{name}: {n} strata in {regions} regions, surveyed in {took:.1} s: {}", layers.join(", "));
}
