//! What a world is made of, found by looking rather than listed: every
//! layer of its generator asked in a cube around spawn (or, drawing
//! nothing there, around each place the world names), things taken
//! apart into parts no wider than a region, sorted into strata: a part or
//! a field, with a kind it draws. The contract checks places of every
//! stratum found (worldgen::checks::contract), however rare, without a list,
//! in regions laid out over several worlds' surveys at once (`regions`),
//! and in each region every stratum surveyed there (`survey_in`).

use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::time::Instant;

use crate::World;
use crate::generator::{
    Children, Field, FieldValues, Generator, Kind, Layer, Node, Op, Population, Shape, across, bounds_of_shapes, each_made, turned,
    unnumbered,
};
use crate::grid::Box3;
use crate::noise::{Rng, hash_text, hash64};
use crate::sites::footprint;
use crate::units::m;
use crate::world::UP;

/** The side of the cube around spawn layers are surveyed in (16 km), lattice units, cut to the world's heights. */
const SIDE: i64 = m(16_000.0);

/**
 * Parts a population's things are taken apart into, about, at most:
 * past it, the cube it is surveyed in stops growing.
 */
const PARTS: usize = 20_000;

/**
 * Cells from a population's centre to its cube's side, at most, once it
 * has found a thing: past it, the cube stops growing too. Each cell
 * asked costs; small cells of rare things (the valley's haystacks) found
 * nothing new past it. One that has found none is looked for up to SIDE,
 * as fields are, so only one drawing nowhere is silent.
 */
const HALF_CELLS: i64 = 512;

/**
 * The level whose columns of sites each field is asked for, in the
 * cube: sites 16 m apart. Fields keep what is finer than the sites in
 * proportion ("Plans" in docs/generators.md), so a narrow one shows.
 */
const FIELD_LEVEL: u32 = 7;

/**
 * Around a place the world names, a field is asked at the columns of a
 * finer level in a smaller cube: sites a metre apart, 128 m. Where the
 * world says a feature is, one narrower than FIELD_LEVEL's sites that it
 * doesn't keep far away (a tunnel's bore) is found there, at any seed.
 */
const PLACE_LEVEL: u32 = 3;
const PLACE_SIDE: i64 = m(128.0);

/**
 * Places of each stratum the survey keeps, at most: those nearest spawn.
 * How many it found is kept apart: the rarest strata get regions first.
 */
const PLACES: usize = 64;

/** Things whose bounds leave the heights that the survey keeps, at most, of each population. */
const LEAVING: usize = 16;

/** A kind a part of things or a field draws, or none if it carves: a stratum, and its places. */
pub struct Stratum {
    /**
     * The layer, in a thing the names of its parts down to this one,
     * numbers left out, and the kind it draws ("towns/church stone",
     * "holes carving"): what it is, at any seed.
     */
    pub name: String,
    /** By index in Generator::kinds. */
    pub kind: Option<Kind>,
    /** How many places it was found at. */
    pub found: usize,
    /** A box around each of the PLACES of them nearest spawn, at most a region wide: parts' bounds, a field's site at its surface. */
    pub places: Vec<Box3>,
}

impl Stratum {
    /** Its places that meet `b`. */
    pub fn places_in(&self, b: &Box3) -> Vec<Box3> {
        self.places.iter().filter(|p| p.overlaps(b)).copied().collect()
    }
}

/** How far a layer was surveyed. */
pub struct Surveyed {
    pub layer: &'static str,
    /** Half the side of the cube around spawn, lattice units. */
    pub half: i64,
    /** Things found, or places a field draws at of those asked. */
    pub found: usize,
    pub seconds: f64,
}

/** A population's things, as their reach and the heights check them (contract::things_hold). */
pub struct Things {
    pub population: &'static str,
    pub reach: [i64; 3],
    pub made: usize,
    /** Each that reaches past its reach: its key, bounds and the box the reach allows. */
    pub past: Vec<(String, Box3, Box3)>,
    /** The first LEAVING whose bounds leave the world's heights: key and bounds. */
    pub leaving: Vec<(String, Box3)>,
}

pub struct Survey {
    pub spawn: [i64; 3],
    /** The widest a region is along any axis, lattice units. */
    pub region: i64,
    pub strata: Vec<Stratum>,
    pub layers: Vec<Surveyed>,
    pub things: Vec<Things>,
}

/**
 * What the survey found of each stratum, by name and kind: at how many
 * places, and the `most` nearest spawn (all, if usize::MAX), whatever
 * order they come in.
 */
struct Found {
    spawn: [i64; 3],
    most: usize,
    /** Of each stratum, at how many places, and those kept, the farthest on top. */
    strata: BTreeMap<(String, Option<Kind>), (usize, BinaryHeap<Near>)>,
}

impl Found {
    fn new(spawn: [i64; 3], most: usize) -> Found {
        Found { spawn, most, strata: BTreeMap::new() }
    }

    /** Whether `place` of the stratum `key` would be kept: at any height, as that comes last (`Near`). */
    fn wants(&self, key: &(String, Option<Kind>), place: &Box3) -> bool {
        let kept = self.strata.get(key).map(|(_, kept)| kept);
        kept.is_none_or(|kept| kept.len() < self.most || kept.peek().is_some_and(|farthest| near(place, self.spawn) < *farthest))
    }

    /** `places` of the stratum `key`, found at `found` places: those of them among its `most` nearest spawn kept. */
    fn add(&mut self, key: (String, Option<Kind>), found: usize, places: impl IntoIterator<Item = Box3>) {
        let (n, kept) = self.strata.entry(key).or_default();
        *n += found;
        for place in places {
            kept.push(near(&place, self.spawn));
            if kept.len() > self.most {
                kept.pop();
            }
        }
    }

    /** What `other` found, as if found here. */
    fn absorb(&mut self, other: Found) {
        for (key, (found, kept)) in other.strata {
            self.add(key, found, kept.into_iter().map(|(.., min, max)| Box3 { min, max }));
        }
    }

    /** The strata, named with the kinds of `generator` they draw. */
    fn into_strata(self, generator: &Generator) -> Vec<Stratum> {
        self.strata
            .into_iter()
            .map(|((name, kind), (found, kept))| {
                let places = kept.into_sorted_vec().into_iter().map(|(.., min, max)| Box3 { min, max }).collect();
                let name = format!("{name} {}", kind.map_or("carving", |k| generator.kinds[k as usize].name));
                Stratum { name, kind, found, places }
            })
            .collect()
    }
}

/**
 * Where a place is among others: how far from spawn along x and z,
 * squared, and where along them; then where it is. Its height comes
 * last, so places of one column rank alike above all others.
 */
type Near = (i64, [i64; 2], [i64; 3], [i64; 3]);

/** Where `b` is among places, nearest `spawn` first. */
fn near(b: &Box3, spawn: [i64; 3]) -> Near {
    let distance = [0, 2].map(|a| ((b.min[a] + b.max[a]) / 2 - spawn[a]).pow(2)).iter().sum();
    (distance, [b.min[0], b.min[2]], b.min, b.max)
}

/**
 * What `world` is made of around spawn, in places at most `region` wide
 * (lattice units), as one of `together` worlds (seeds of one), which
 * share the fields' SIDE square and PARTS (docs/checks.md): all of it,
 * or what `only` names (`within`).
 */
pub fn survey(world: &World, together: usize, region: i64, only: &[&str]) -> Survey {
    let mut found = Found::new(world.spawn, PLACES);
    let (mut layers, mut things) = (Vec::new(), Vec::new());
    let wanted = || wanted(world, only);
    let fields: Vec<&Field> = wanted().filter_map(|layer| if let Layer::Field(field) = layer { Some(field) } else { None }).collect();
    let cube = |centre, side| world.within(Box3::around(centre, side / 2));
    let side = (SIDE as f64 / (together as f64).sqrt()) as i64;
    let mut at_spawn = survey_fields(world, &cube(world.spawn, side), FIELD_LEVEL, &fields, &mut found).into_iter();
    for layer in wanted() {
        let start = Instant::now();
        match layer {
            Layer::Population(population) => {
                let mut things_around = |centre| survey_things(world, centre, population, PARTS / together, region, only, &mut found);
                let at_spawn = things_around(world.spawn);
                let (half, seen) = or_at_places(world, at_spawn, things_around, |s| s.1.made);
                layers.push(Surveyed { layer: population.name, half, found: seen.made, seconds: start.elapsed().as_secs_f64() });
                things.push(seen);
            }
            Layer::Field(field) => {
                let (at_spawn, seconds) = at_spawn.next().expect("a field");
                let at_place = |centre| survey_fields(world, &cube(centre, PLACE_SIDE), PLACE_LEVEL, &[field], &mut found)[0].0;
                let drawn = or_at_places(world, at_spawn, at_place, |drawn| *drawn);
                layers.push(Surveyed { layer: field.name, half: side / 2, found: drawn, seconds: seconds + start.elapsed().as_secs_f64() });
            }
        }
    }
    let mut strata = found.into_strata(&world.generator);
    strata.retain(|stratum| within(only, &stratum.name));
    Survey { spawn: world.spawn, region, strata, layers, things }
}

/**
 * `at_spawn`, what a layer's survey found around spawn, or, if nothing
 * (`found` is 0), what `survey` finds around each place the world names
 * in turn: a feature too rare to be near spawn at every seed (the
 * valley's tunnels) is where the world says it is.
 */
fn or_at_places<T>(world: &World, at_spawn: T, mut survey: impl FnMut([i64; 3]) -> T, found: impl Fn(&T) -> usize) -> T {
    let mut done = at_spawn;
    for (_, find) in &world.points {
        if found(&done) > 0 {
            break;
        }
        done = survey(find());
    }
    done
}

/**
 * Where each of `fields` draws in `b`, asked at every column of sites of
 * `level` whose sites meet it, at a height in it, into `found`: all of them at a
 * column before the next, so a plan they share (a world's land) is made
 * there once. Rows nearest spawn go first, and in each the columns: any
 * order keeps the same places (`Found`), this one finds them early, so
 * few later are asked at level 0 (`survey_site`). Returns at how many
 * each draws, and how long it took: a shared plan's time is the first's
 * to read it.
 */
fn survey_fields(world: &World, b: &Box3, level: u32, fields: &[&Field], found: &mut Found) -> Vec<(usize, f64)> {
    let seeds: Vec<u64> = fields.iter().map(|field| hash_text(field.name, world.generator.seed)).collect();
    let s = 1i64 << level;
    let (h0, h1) = (b.min[UP], b.max[UP]);
    // Column i's sites reach s / 2 either side of i·s.
    let columns = |a: usize| {
        let mut columns: Vec<i64> = ((b.min[a] + s / 2).div_euclid(s)..(b.max[a] + s / 2 + s - 1).div_euclid(s)).map(|i| i * s).collect();
        columns.sort_by_key(|v| (v - found.spawn[a]).abs());
        columns
    };
    let [is, ks] = across(UP).map(columns);
    let mut drawn = vec![(0, 0.0); fields.len()];
    for (&i, &k) in ks.iter().flat_map(|k| is.iter().map(move |i| (i, k))) {
        // A site's column of the level, as generating it asks: at any height, for a field of sites; a hash of where it is, in any order.
        for ((field, seed), drawn) in fields.iter().zip(&seeds).zip(&mut drawn) {
            let start = Instant::now();
            let p = turned(UP, [i, k], h0 + Rng::new(hash64(*seed, i, 0, k)).below((h1 - h0) as u32) as i64);
            drawn.0 += survey_site(field, p, s, found) as usize;
            drawn.1 += start.elapsed().as_secs_f64();
        }
    }
    drawn
}

/**
 * What `field` draws at `p`, a site of sites `s` apart, into `found`;
 * whether anything. Where a place of a kind it draws there is to be kept
 * (`Found::wants`), it is asked at level 0 too: for what it draws finer
 * than these sites, and where: a coarse site's span is stretched to keep
 * a site (ground's column.rs), so its top may be far from the ground's.
 */
fn survey_site(field: &Field, p: [i64; 3], s: i64, found: &mut Found) -> bool {
    let carves = field.op == Op::Carve;
    let key = |kind: Kind| (field.name.to_string(), (!carves).then_some(kind));
    // The site, moved along its axis to the top of what fills, the middle of what carves: fields whose surfaces are near share a region.
    let place = |&(_, along, from, to): &Drawn| {
        let mut centre = p;
        centre[along] = if carves { (from + to) / 2 } else { to };
        Box3::around(centre, s / 2)
    };
    let Some(coarse) = drawn(field, p, s) else { return false };
    let there = place(&coarse);
    let fine = coarse.0.iter().any(|&kind| found.wants(&key(kind), &there)).then(|| drawn(field, p, 1)).flatten();
    let place = place(fine.as_ref().unwrap_or(&coarse));
    let keys: BTreeSet<_> = coarse.0.iter().chain(fine.iter().flat_map(|f| &f.0)).map(|&kind| key(kind)).collect();
    keys.into_iter().for_each(|key| found.add(key, 1, [place]));
    true
}

/** What a field draws at a site: its kinds, and along an axis, from and to. */
type Drawn = (Vec<Kind>, usize, i64, i64);

/** What `field` draws at `p`, as a site of sites `s` apart reads it. */
fn drawn(field: &Field, p: [i64; 3], s: i64) -> Option<Drawn> {
    match &field.values {
        FieldValues::At(at) => footprint(s, [0; 3], || at(p[0], p[1], p[2])).map(|(kind, _)| (vec![kind], UP, p[UP], p[UP] + 1)),
        FieldValues::Shapes(near) => {
            // A shape reaching the site's voxel, from its bottom to its top.
            let b = Box3::around(p, s);
            let shapes = footprint(s, [0; 3], || near(&b));
            let shape = shapes.iter().find(|shape| shape.bounds.overlaps(&b))?;
            Some((vec![shape.kind], UP, shape.bounds.min[UP], shape.bounds.max[UP]))
        }
        FieldValues::Spans { axis, spans } => {
            let [a, b] = across(*axis);
            let spans = footprint(s, [0; 3], || spans(p[a], p[b]));
            let (from, to) = (spans.iter().map(|s| s.from).min()?, spans.iter().map(|s| s.to).max()?);
            Some((spans.iter().map(|s| s.kind).collect(), *axis, from, to))
        }
    }
}

/**
 * The things of `population` in a cube around `centre`, from 16 cells
 * wide, twice as wide each time while they are taken apart into fewer
 * than `parts` / 4 parts, up to SIDE (HALF_CELLS once any is found); their
 * parts, into `found`. Returns half the cube's side, and the things.
 */
fn survey_things(
    world: &World,
    centre: [i64; 3],
    population: &Population,
    parts: usize,
    region: i64,
    only: &[&str],
    found: &mut Found,
) -> (i64, Things) {
    let (n, reach) = (population.cell, population.reach);
    let mut half = 8;
    loop {
        let mut taken = Found::new(world.spawn, PLACES);
        let mut seen = Things { population: population.name, reach, made: 0, past: Vec::new(), leaving: Vec::new() };
        let cube = world.within(Box3::around(centre, half * n));
        let count = things_in(world, population, &cube, region, only, &mut taken, |key, bounds, allowed| {
            seen.made += 1;
            if !bounds.inside(&allowed) {
                seen.past.push((key.clone(), bounds, allowed));
            }
            let leaves = world.heights.is_some_and(|(h0, h1)| bounds.min[UP] < h0 || bounds.max[UP] > h1);
            if leaves && seen.leaving.len() < LEAVING {
                seen.leaving.push((key, bounds));
            }
        });
        let most = if seen.made > 0 { HALF_CELLS } else { i64::MAX }.min(SIDE / 2 / n);
        if count * 4 > parts || half >= most {
            found.absorb(taken);
            return (half * n, seen);
        }
        half = (2 * half).min(most);
    }
}

/**
 * The things of `population` that may reach `b`, each to
 * `each` (its key, bounds and the box its reach allows), taken apart into
 * `parts` (`take_apart`): how many.
 */
fn things_in(
    world: &World,
    population: &Population,
    b: &Box3,
    region: i64,
    only: &[&str],
    parts: &mut Found,
    mut each: impl FnMut(String, Box3, Box3),
) -> usize {
    let mut count = 0;
    each_made(&world.generator, population, b, |key, bounds, allowed, node| {
        count += take_apart(node, population.name.to_string(), region, only, parts);
        each(key, bounds, allowed);
    });
    count
}

/**
 * The strata of `world`, or those `only` names, in `b`, each with every
 * place of it that meets `b`: what `survey` finds there, none left out
 * for being far from spawn.
 */
pub fn survey_in(world: &World, b: &Box3, region: i64, only: &[&str]) -> Vec<Stratum> {
    let mut found = Found::new(world.spawn, usize::MAX);
    let wanted = || wanted(world, only);
    let fields: Vec<&Field> = wanted().filter_map(|layer| if let Layer::Field(field) = layer { Some(field) } else { None }).collect();
    survey_fields(world, b, FIELD_LEVEL, &fields, &mut found);
    for layer in wanted() {
        if let Layer::Population(population) = layer {
            things_in(world, population, b, region, only, &mut found, |_, _, _| {});
        }
    }
    let mut strata = found.into_strata(&world.generator);
    strata.iter_mut().for_each(|stratum| stratum.places.retain(|place| place.overlaps(b)));
    strata.retain(|stratum| within(only, &stratum.name) && !stratum.places.is_empty());
    strata
}

/**
 * `node`, named `name`, into parts at most `region` wide, and down to
 * those `only` names: itself if it is one, else its own shapes and its
 * children's parts, each named by the names down to it; each a place of
 * every kind it draws. Returns how many parts.
 */
fn take_apart(node: &Node, name: String, region: i64, only: &[&str], parts: &mut Found) -> usize {
    let bounds = node.bounds();
    // Whether `only` names a part of it (`covers`, after a '/').
    let named_within = only.iter().any(|o| o.strip_prefix(name.as_str()).is_some_and(|rest| rest.starts_with('/')));
    if (fits(&bounds, region) && !named_within) || node.children.is_none() {
        let mut kinds = BTreeSet::new();
        kinds_of(node, &mut kinds);
        let place = centred(&bounds, region);
        kinds.into_iter().for_each(|kind| parts.add((name.clone(), Some(kind)), 1, [place]));
        return 1;
    }
    let own = node.shapes.get();
    let mut count = 0;
    if own.iter().any(|s| s.op != Op::Carve) {
        let place = centred(&bounds_of_shapes(own.iter().map(|s| &s.bounds)), region);
        let kinds: BTreeSet<Kind> = shapes_kinds(own).collect();
        kinds.into_iter().for_each(|kind| parts.add((format!("{name} itself"), Some(kind)), 1, [place]));
        count += 1;
    }
    each_child(node, |child, node| count += take_apart(node, format!("{name}/{}", unnumbered(child)), region, only, parts));
    count
}

/** Whether `stratum` is the one named `name`, or a part (after a '/') or a kind (after a space) of it. */
pub(crate) fn covers(name: &str, stratum: &str) -> bool {
    stratum == name || stratum.strip_prefix(name).is_some_and(|rest| rest.starts_with([' ', '/']))
}

/**
 * Whether `stratum` is in what `only` names (`covers`): layers, things'
 * parts or kinds of them ("towns/block/church", "railway ballast"); all,
 * if it names none.
 */
fn within(only: &[&str], stratum: &str) -> bool {
    only.is_empty() || only.iter().any(|o| covers(o, stratum))
}

/**
 * The layers of `world` holding what `only` names, or in it ("railway in
 * its tunnels" in "railway"): all, if it names none.
 */
fn wanted<'a>(world: &'a World, only: &'a [&str]) -> impl Iterator<Item = &'a Layer> {
    let on_one_line = |layer: &str, o: &str| covers(layer, o) || covers(o, layer);
    world.generator.layers.iter().filter(move |layer| only.is_empty() || only.iter().any(|o| on_one_line(layer.name(), o)))
}

/** Each of a node's children, with its name: all of those it makes for a box, made for its bounds. */
fn each_child(node: &Node, mut each: impl FnMut(&str, &Node)) {
    match &node.children {
        None => {}
        Some(Children::Fixed(children)) => children.iter().for_each(|c| each(&c.name, &c.node)),
        Some(Children::Reaching(reaching)) => reaching(&node.bounds()).iter().for_each(|c| each(&c.name, &c.node)),
    }
}

/** The kinds a node and everything in it draws, near or far. */
fn kinds_of(node: &Node, kinds: &mut BTreeSet<Kind>) {
    kinds.extend(shapes_kinds(node.shapes.get()));
    if let Some(far) = &node.far {
        kinds.extend(shapes_kinds(far.shapes()));
    }
    each_child(node, |_, child| kinds_of(child, kinds));
}

fn shapes_kinds(shapes: &[Shape]) -> impl Iterator<Item = Kind> + '_ {
    shapes.iter().filter(|s| s.op != Op::Carve).map(|s| s.kind)
}

/** Whether `b` is at most `region` wide along every axis. */
fn fits(b: &Box3, region: i64) -> bool {
    (0..3).all(|i| b.max[i] - b.min[i] <= region)
}

/** `b`, at most `region` wide along each axis about its centre. */
fn centred(b: &Box3, region: i64) -> Box3 {
    let c = [0, 1, 2].map(|i| (b.min[i] + b.max[i]).div_euclid(2));
    Box3 { min: [0, 1, 2].map(|i| b.min[i].max(c[i] - region / 2)), max: [0, 1, 2].map(|i| b.max[i].min(c[i] + region / 2)) }
}

/**
 * Regions a stratum is held in, at least, while it has places no region
 * holds: in one, its mean is of one setting (docs/checks.md). Each costs
 * about a region of its own.
 */
pub const HELD: usize = 3;

/**
 * Regions over all of `surveys` holding places of each of their strata,
 * one by name in all of them: the rarest first, each until HELD regions
 * hold it. A free place of it joins the region it widens least, or else
 * the one nearest its spawn starts one. Each with the strata laid out in it.
 */
pub fn regions(surveys: &[Survey]) -> Vec<(Placed, Vec<String>)> {
    // Each stratum by name: how many places it was found at in all, and its places in each survey.
    let mut strata: BTreeMap<&str, (usize, Vec<Placed>)> = BTreeMap::new();
    for (s, survey) in surveys.iter().enumerate() {
        for stratum in &survey.strata {
            let (found, places) = strata.entry(&stratum.name).or_default();
            *found += stratum.found;
            places.extend(stratum.places.iter().map(|place| (s, *place)));
        }
    }
    let mut order: Vec<_> = strata.into_iter().collect();
    order.sort_by_key(|(name, (found, _))| (*found, *name));
    let mut regions: Vec<(Placed, Vec<String>)> = Vec::new();
    let meets = |(s, place): &Placed, (of, region): &Placed| s == of && place.overlaps(region);
    let volume = |b: &Box3| (0..3).map(|i| (b.max[i] - b.min[i]) as f64).product::<f64>();
    for (name, (_, places)) in order {
        loop {
            let holds = |region: &Placed| places.iter().any(|place| meets(place, region));
            let free: Vec<&Placed> = places.iter().filter(|place| !regions.iter().any(|(region, _)| meets(place, region))).collect();
            if free.is_empty() || regions.iter().filter(|(region, _)| holds(region)).count() >= HELD {
                break;
            }
            let mut best: Option<(f64, usize, Box3)> = None;
            for (s, place) in &free {
                for (r, ((_, region), _)) in regions.iter().enumerate().filter(|(_, (region, _))| region.0 == *s && !holds(region)) {
                    let union = Box3 {
                        min: [0, 1, 2].map(|a| region.min[a].min(place.min[a])),
                        max: [0, 1, 2].map(|a| region.max[a].max(place.max[a])),
                    };
                    let growth = volume(&union) - volume(region);
                    if fits(&union, surveys[*s].region) && best.as_ref().is_none_or(|(g, ..)| growth < *g) {
                        best = Some((growth, r, union));
                    }
                }
            }
            match best {
                Some((_, r, union)) => {
                    regions[r].0.1 = union;
                    regions[r].1.push(name.to_string());
                }
                None => {
                    let &place = free.into_iter().min_by_key(|(s, place)| near(place, surveys[*s].spawn)).expect("a place");
                    regions.push((place, vec![name.to_string()]));
                }
            }
        }
    }
    regions
}

/** A box in one of several surveys, by index: a place, or a region. */
pub type Placed = (usize, Box3);

impl Survey {
    /** The kinds of `generator` that nothing surveyed draws, by name. */
    pub fn unseen(&self, generator: &Generator) -> Vec<&'static str> {
        let seen: BTreeSet<Kind> = self.strata.iter().filter_map(|s| s.kind).collect();
        generator.kinds.iter().enumerate().filter(|(i, _)| !seen.contains(&(*i as Kind))).map(|(_, k)| k.name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * In any order the same places are kept, and none that was not wanted
     * when it came: so a column whose place isn't wanted need not be asked
     * at level 0 (`survey_site`).
     */
    #[test]
    fn found_keeps_the_same_places_in_any_order_and_none_unwanted() {
        let key = || ("stratum".to_string(), None);
        let columns = (0..32).flat_map(|z| (0..32).map(move |x| [x, hash64(1, x, 0, z) as i64 % 64, z]));
        let places: Vec<Box3> = columns.map(|c| Box3::around(c, 1)).collect();
        let kept = |order: u64| {
            let mut places = places.clone();
            places.sort_by_key(|b| hash64(order, b.min[0], b.min[1], b.min[2]));
            let (mut found, mut unwanted) = (Found::new([16, 0, 16], 16), Vec::new());
            for place in places {
                if !found.wants(&key(), &place) {
                    unwanted.push(place);
                }
                found.add(key(), 1, [place]);
            }
            let kept = found.into_strata(&Generator { kinds: Vec::new(), seed: 0, layers: Vec::new() }).remove(0).places;
            assert!(!unwanted.iter().any(|p| kept.contains(p)), "order {order}: kept a place it did not want");
            kept
        };
        (1..8).for_each(|order| assert_eq!(kept(order), kept(0), "order {order}"));
    }
}
