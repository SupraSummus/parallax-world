//! The contract every world keeps (docs/checks.md), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; nothing lies outside its heights, things keep their
//! reach, and what is skipped where it says it may not draw draws nothing
//! there; and on average over each stratum's regions, each level's cells
//! in its places look as what is in them does, the level two finer
//! (checks::cells). What
//! is seen is a `Look`: linear, as a coarse site averages finer ones, so
//! the sum of each kind's, which tell what changed.

use std::collections::BTreeMap;
use std::ops::Range;
use std::time::Instant;

use crate::World;
use crate::checks::cells;
use crate::checks::cells::FINER_BY;
use crate::checks::survey::{Survey, nearest_first, survey};
use crate::generator::{Generator, Whole, checking_skipped, generate, kind_of};
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::noise::Rng;
use crate::units::m;

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

/**
 * Where `level`'s cells of `region` are sampled: `region` and a site of
 * the level around it, so a cell reaching into it is whole in it (a
 * thing's walls are its region's), grown to at least 16 sites across.
 */
pub(crate) fn around(region: &Box3, level: u32) -> Box3 {
    let region = region.widen(1 << level);
    let grow = |i: usize| 0.max((8i64 << level) - (region.max[i] - region.min[i]) / 2);
    Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) }
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
 * solid's. A look that is no number is no number apart, which is within
 * nothing (`Margin::broken`, `breaks`).
 */
pub fn apart(a: &Look, c: &Look, channels: Range<usize>) -> f64 {
    let colour = |l: &Look, i: usize| if i == 0 { l[0] } else { l[i] / l[0] };
    let coloured = a[0] >= SEEN && c[0] >= SEEN;
    // f64::max would drop a NaN.
    channels
        .filter(|&i| i == 0 || coloured)
        .map(|i| (colour(c, i) - colour(a, i)).abs())
        .fold(0.0, |most, d| if d <= most { most } else { d })
}

/** How far a level's cells in a stratum's places in a region look from what is in them (checks::cells), on average. */
#[derive(Clone, Debug)]
pub struct Margin {
    pub region: Box3,
    pub stratum: String,
    pub level: u32,
    /** The level it is measured against: FINER_BY finer, down to 0. */
    pub against: u32,
    /** What is seen at `against` and at `level`, on average. */
    pub looks: [Look; 2],
    /** How far, on average, each of its cells is from what is in it, in its channel furthest. */
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
        !(self.difference() < within.apart && self.spread < within.spread)
    }
}

/** A world's margins in its regions, to print. */
#[derive(Clone)]
pub struct Report {
    pub name: String,
    pub margins: Vec<Margin>,
}

impl Report {
    fn new(name: &str) -> Report {
        Report { name: name.to_string(), margins: Vec::new() }
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
                writeln!(f, "{} in x {}..{}, y {}..{}, z {}..{}:", self.name, r.min[0], r.max[0], r.min[1], r.max[1], r.min[2], r.max[2])?;
                region = Some(r);
            }
            let [a, c] = m.looks.map(|v| look(&v));
            let (stratum, level, against, spread, apart) = (&m.stratum, m.level, m.against, m.spread, m.difference());
            writeln!(f, "  {stratum}, level {level} against {against}, spread {spread:.3}: {a} -> {c}: {apart:.3} apart")?;
        }
        Ok(())
    }
}

/**
 * The area of `region` a level's grid covers: where its cells are
 * sampled (`around`), and those of the level FINER_BY coarser, held
 * against it (checks::cells).
 */
fn area_at(region: &Box3, level: u32, levels: u32) -> Box3 {
    around(region, (level + FINER_BY).min(levels).max(level))
}

/**
 * A region the contract checks in, and the strata it holds, each by name
 * with its places in it: a stratum's cells are those in its places.
 */
#[derive(Clone, Debug)]
pub struct Region {
    pub area: Box3,
    pub strata: Vec<(String, Vec<Box3>)>,
}

impl Region {
    /** `area`, all of it the place of one stratum, named `name`. */
    pub fn whole(name: &str, area: Box3) -> Region {
        Region { area, strata: vec![(name.to_string(), vec![area])] }
    }
}

/**
 * `world`, named `name` (with its seed, say), in each of `regions`
 * (`hold_region`) up to `levels`: how each level's cells look against
 * what is in them, and where it breaks the rules that hold exactly.
 */
pub fn in_regions(name: &str, world: &World, regions: &[Region], levels: u32) -> (Report, Vec<String>) {
    let mut report = Report::new(name);
    let mut broken = Vec::new();
    for region in regions {
        let (margins, b) = hold_region(world, region, levels);
        report.margins.extend(margins);
        broken.extend(b);
    }
    (report, broken)
}

/**
 * `region` of `world`, each level from 0 to one past `levels` generated
 * once: in parts (`parts_hold`), and up to `levels`, each stratum's cells
 * against the level FINER_BY finer (checks::cells). Its margins, and
 * where it breaks the rules that hold exactly.
 */
fn hold_region(world: &World, region: &Region, levels: u32) -> (Vec<Margin>, Vec<String>) {
    let (mut margins, mut broken) = (Vec::new(), Vec::new());
    // Each level's grid, until the level FINER_BY coarser is held against it.
    let mut grids: Vec<Option<VoxelGrid>> = Vec::new();
    for level in 0..=levels + 1 {
        let grid = generate(&world.generator, &box_at(&area_at(&region.area, level, levels), level), level);
        broken.extend(parts_hold(&world.generator, &grid, level));
        grids.push(Some(grid));
        if (1..=levels).contains(&level) {
            let against = level.saturating_sub(FINER_BY);
            let [Some(fine), Some(coarse)] = [against, level].map(|l| grids[l as usize].as_ref()) else { unreachable!("kept") };
            let each = cells::cells_of([fine, coarse], &region.strata, &around(&region.area, level), [against, level]);
            for ((stratum, _), cells) in region.strata.iter().zip(each).filter(|(_, cells)| !cells.is_empty()) {
                let n = cells.len() as f64;
                let looks = [0, 1].map(|i| std::array::from_fn(|k| cells.iter().map(|cell| cell[i][k]).sum::<f64>() / n));
                let spread = cells.iter().map(cells::deviation).sum::<f64>() / n;
                margins.push(Margin { region: region.area, stratum: stratum.clone(), level, against, looks, spread });
            }
        }
        if level >= FINER_BY {
            grids[(level - FINER_BY) as usize] = None;
        }
    }
    (margins, broken)
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

/** Sites a side of the parts `parts_hold` generates a box in: small, so lines and things are skipped near what they draw. */
const PART: i64 = 32;

/**
 * `whole`, a box generated at `level`, generated again in parts PART
 * sites a side, each alone and with what it skips drawn all the same
 * (`checking_skipped`): each part equals it whole (`VoxelGrid::held`),
 * what is skipped draws nothing, and a part `whole` says is empty or
 * buried is (a site around it too, if buried). Returns where not.
 */
pub fn parts_hold(generator: &Generator, whole: &VoxelGrid, level: u32) -> Vec<String> {
    let b = whole.clip();
    let mut broken = Vec::new();
    if whole.pieces.alpha.iter().all(|a| *a == 0) && whole.medium.alpha.iter().all(|a| *a == [0; 3]) {
        broken.push(format!("level {level}: nothing drawn in {b:?}, for parts to differ from"));
    }
    let mut random = Rng::new(7 + level as u64);
    let offset: [i64; 3] = std::array::from_fn(|_| random.below(PART as u32) as i64);
    let starts = |a: usize| (b.min[a] - offset[a]..b.max[a]).step_by(PART as usize);
    for z in starts(2) {
        for y in starts(1) {
            for x in starts(0) {
                let part = Box3 { min: [x, y, z], max: [x + PART, y + PART, z + PART] }.intersect(&b);
                let (grid, drew) = checking_skipped(|| generate(generator, &part.widen(1), level));
                broken.extend(drew.into_iter().map(|d| format!("level {level}: {d}")));
                if let Some(site) = sites(&part).find(|&(x, y, z)| grid.held(x, y, z) != whole.held(x, y, z)) {
                    broken.push(format!("level {level}: part {part:?} generated alone differs at {site:?} from {b:?} generated whole"));
                }
                let Some(whole) = crate::generator::whole(generator, &part, level) else { continue };
                let unlike = match whole {
                    Whole::Empty => sites(&part).find(|&(x, y, z)| grid.alpha(x, y, z) > 0),
                    Whole::Buried(_) => sites(&part.widen(1)).find(|&(x, y, z)| grid.alpha(x, y, z) < ALPHA),
                };
                if let Some(site) = unlike {
                    broken.push(format!("level {level}: {part:?} is {whole:?}, as its layers say, but not at site {site:?}"));
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

/**
 * The widest region the contract checks in (32 m), along any axis: each
 * holds places of strata (worldgen::checks::survey), as many as fit.
 */
pub const REGION: i64 = m(32.0);

/**
 * What `world` is made of (`survey`), and regions holding a place of
 * each of its strata, by name with their places; `only` those of strata whose
 * names hold one of `only`, if any (a layer's name first:
 * "towns/church", "railway"), and only their layers surveyed.
 */
pub fn regions(world: &World, only: &[&str]) -> (Survey, Vec<Region>) {
    let survey = survey(world, REGION, |layer| only.is_empty() || only.iter().any(|o| o.starts_with(layer) || layer.starts_with(o)));
    let names: Vec<String> = (0..survey.strata.len()).map(|i| survey.describe(&world.generator, i)).collect();
    let wanted = |i: &usize| only.is_empty() || only.iter().any(|o| names[*i].contains(o));
    let regions = survey.regions(wanted).into_iter();
    let regions = regions.map(|(area, strata)| Region {
        strata: strata.into_iter().map(|i| (names[i].clone(), survey.places_in(i, &area))).collect(),
        area,
    });
    let regions = regions.collect();
    (survey, regions)
}

/** What `check` found of a world at one seed. */
pub struct Checked {
    pub name: String,
    /** Each region and the strata it holds. */
    pub regions: Vec<Region>,
    /** The margins of every stratum in every region. */
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
 * margins (by level) averaged over the regions holding it
 * at every seed, if they are as far apart or as spread as it is to keep
 * it (as `declared` by name, `Accuracy::of`), or more; and, if every
 * stratum was checked, a name declared that covers none.
 * Each broken mean names the regions it was averaged over, and the
 * worst of them by channel. Each with what it is of.
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
    // Each stratum's margins, by level, from every world checked.
    let mut means: BTreeMap<(&str, u32), Vec<Held>> = BTreeMap::new();
    for c in checked {
        for m in &c.report.margins {
            means.entry((&m.stratum, m.level)).or_default().push((c, m));
        }
    }
    if checked.iter().all(|c| c.complete) {
        for (name, _) in declared.iter().filter(|(name, _)| !means.keys().any(|(s, ..)| covers(name, s))) {
            push(None, format!("an accuracy is declared for {name}, which no stratum surveyed is or is in"));
        }
    }
    for ((stratum, level), margins) in means {
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
            "{stratum}, level {level}: {apart:.3} apart, spread {spread:.3} (within {}, {}), on average over {} regions (stops {:+.3}, shows {:+.3} {:+.3} {:+.3}):",
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
 * The whole contract for `world`, named `name` (with its seed, say), in
 * regions holding everything it is made of around spawn (`regions`),
 * `only` some if any, up to the level `levels` its cells look as what
 * is in them to. Prints its regions and margins (see them with
 * --nocapture); returns where it breaks it.
 */
pub fn check(name: &str, world: &World, levels: u32, only: &[&str]) -> Checked {
    let start = Instant::now();
    let (survey, regions) = regions(world, only);
    print_survey(name, &survey, regions.len(), start);
    if only.is_empty() {
        // Not checked: a world may share a table of kinds with others (the moor's is the valley's).
        println!("{name}: kinds nothing surveyed draws: {}", survey.unseen(&world.generator).join(", "));
    }
    let boxes: Vec<Box3> = regions.iter().map(|r| r.area).collect();
    let start = Instant::now();
    let (report, mut broken) = in_regions(name, world, &regions, levels);
    let held = start.elapsed().as_secs_f64();
    broken.extend(heights_hold(world, &boxes, levels));
    broken.extend(things_hold(world, &survey));
    let rest = start.elapsed().as_secs_f64() - held;
    println!("{report}{name}: regions {held:.1} s, heights and things {rest:.1} s");
    let silent = survey.layers.iter().filter(|l| l.found == 0).map(|l| l.layer).collect();
    Checked { name: name.to_string(), regions, report, broken, silent, complete: only.is_empty() }
}

/**
 * The contract for a `component` of the world `make` makes (`World::alone`),
 * at its `places` places nearest spawn as the survey finds it, none over
 * another, a region each and a thread each, up to `levels` (`hold_region`):
 * its cells in its places, all around it drawn as the world draws it.
 * Its margins are the component's, by name. Alone if `make` makes the
 * world cut to it.
 */
pub fn check_component(name: &str, make: &dyn Fn() -> World, component: &str, places: usize, levels: u32) -> Checked {
    let start = Instant::now();
    let (world, focus) = (make(), make().alone(component));
    let survey = survey(&focus, REGION, |_| true);
    let in_it = |b: &Box3| (0..survey.strata.len()).flat_map(|i| survey.places_in(i, b)).collect();
    let regions: Vec<Region> =
        nearest(&survey, places).iter().map(|b| Region { area: *b, strata: vec![(component.to_string(), in_it(b))] }).collect();
    print_survey(name, &survey, regions.len(), start);
    let start = Instant::now();
    let each: Vec<(Vec<Margin>, Vec<String>)> = std::thread::scope(|scope| {
        let threads: Vec<_> = regions
            .iter()
            .map(|r| {
                let world = &world;
                scope.spawn(move || {
                    let (margins, mut broken) = hold_region(world, r, levels);
                    broken.extend(heights_hold(world, &[r.area], levels));
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
    let report = Report { name: name.to_string(), margins: margins.into_iter().flatten().collect() };
    println!("{report}{name}: checked in {:.1} s", start.elapsed().as_secs_f64());
    let silent = survey.layers.iter().filter(|l| l.found == 0).map(|l| l.layer).collect();
    Checked { name: name.to_string(), regions, report, broken, silent, complete: true }
}

/** Of the places of `survey`'s strata, the `n` nearest its spawn (along x and z), none overlapping one nearer. */
fn nearest(survey: &Survey, n: usize) -> Vec<Box3> {
    let mut places: Vec<Box3> = survey.strata.iter().flat_map(|s| s.places.iter().copied()).collect();
    nearest_first(&mut places, survey.spawn);
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
