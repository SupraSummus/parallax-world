//! What a world is made of, found by looking rather than listed: every
//! layer of its generator asked in a cube around spawn (or, drawing
//! nothing there, around each place the world names), things taken
//! apart into parts no wider than a region, sorted into strata: a part or
//! a field, with a kind it draws. The contract checks a place of every
//! stratum found (worldgen::checks::contract), however rare, without a list.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::World;
use crate::generator::{
    Children, Field, FieldValues, Generator, Kind, Layer, Node, Op, Population, Shape, bounds_of_shapes, each_made, unnumbered,
};
use crate::grid::Box3;
use crate::noise::{Rng, hash_text};
use crate::plan::footprint;
use crate::units::m;

/** The side of the cube around spawn layers are surveyed in (16 km), lattice units, cut to the world's heights. */
const SIDE: i64 = m(16_000.0);

/**
 * Parts a population's things are taken apart into, about, at most:
 * past it, the cube it is surveyed in stops growing.
 */
const PARTS: usize = 20_000;

/**
 * Cells from a population's centre to its cube's side, at most: past
 * it, the cube stops growing too. Each cell asked costs; small cells of
 * rare things (the valley's haystacks) found nothing new past it.
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
    /** The layer, and in a thing the names of its parts down to this one, numbers left out. */
    pub name: String,
    /** By index in Generator::kinds. */
    pub kind: Option<Kind>,
    /** How many places it was found at. */
    pub found: usize,
    /** A box around each of the PLACES of them nearest spawn, at most a region wide: parts' bounds, a field's site at its surface. */
    pub places: Vec<Box3>,
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

/** What the survey found of each stratum, by name and kind: at how many places, and the PLACES nearest spawn. */
struct Found {
    spawn: [i64; 3],
    strata: BTreeMap<(String, Option<Kind>), (usize, Vec<Box3>)>,
}

impl Found {
    fn new(spawn: [i64; 3]) -> Found {
        Found { spawn, strata: BTreeMap::new() }
    }

    /** `places` of the stratum `key`, found at `found` places: those of them among its PLACES nearest spawn kept. */
    fn add(&mut self, key: (String, Option<Kind>), found: usize, places: impl IntoIterator<Item = Box3>) {
        let (n, kept) = self.strata.entry(key).or_default();
        *n += found;
        for place in places {
            kept.push(place);
            if kept.len() == 2 * PLACES {
                nearest_first(kept, self.spawn);
                kept.truncate(PLACES);
            }
        }
    }

    fn into_strata(self) -> Vec<Stratum> {
        let spawn = self.spawn;
        self.strata
            .into_iter()
            .map(|((name, kind), (found, mut places))| {
                nearest_first(&mut places, spawn);
                places.truncate(PLACES);
                Stratum { name, kind, found, places }
            })
            .collect()
    }
}

/** How far `b` is from `spawn`, along x and z, squared. */
fn distance(b: &Box3, spawn: [i64; 3]) -> i64 {
    [0, 2].map(|a| ((b.min[a] + b.max[a]) / 2 - spawn[a]).pow(2)).iter().sum()
}

/** `places` nearest `spawn` first (along x and z), then by where they are. */
pub(crate) fn nearest_first(places: &mut [Box3], spawn: [i64; 3]) {
    places.sort_by_key(|b| (distance(b, spawn), b.min, b.max));
}

/** What `world` is made of around spawn, in places at most `region` wide (lattice units): of the layers `wanted` by name. */
pub fn survey(world: &World, region: i64, wanted: impl Fn(&str) -> bool) -> Survey {
    let mut found = Found::new(world.spawn);
    let (mut layers, mut things) = (Vec::new(), Vec::new());
    let wanted = || world.generator.layers.iter().filter(|layer| wanted(layer.name()));
    let fields: Vec<&Field> = wanted().filter_map(|layer| if let Layer::Field(field) = layer { Some(field) } else { None }).collect();
    let mut at_spawn = survey_fields(world, world.spawn, (FIELD_LEVEL, SIDE), &fields, &mut found).into_iter();
    for layer in wanted() {
        let start = Instant::now();
        match layer {
            Layer::Population(population) => {
                let mut things_around = |centre| survey_things(world, centre, population, region, &mut found);
                let at_spawn = things_around(world.spawn);
                let (half, seen) = or_at_places(world, at_spawn, things_around, |s| s.1.made);
                layers.push(Surveyed { layer: population.name, half, found: seen.made, seconds: start.elapsed().as_secs_f64() });
                things.push(seen);
            }
            Layer::Field(field) => {
                let (at_spawn, seconds) = at_spawn.next().expect("a field");
                let at_place = |centre| survey_fields(world, centre, (PLACE_LEVEL, PLACE_SIDE), &[field], &mut found)[0].0;
                let drawn = or_at_places(world, at_spawn, at_place, |drawn| *drawn);
                layers.push(Surveyed { layer: field.name, half: SIDE / 2, found: drawn, seconds: seconds + start.elapsed().as_secs_f64() });
            }
        }
    }
    let strata = found.into_strata();
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
 * Where each of `fields` draws around `centre`, asked at every column of
 * sites of `level` in a cube `side` wide, at a height in it, into
 * `found`: all of them at a column before the next, so a plan they share
 * (a world's land) is made there once. Returns at how many each draws,
 * and how long it took: a shared plan's time is the first's to read it.
 */
fn survey_fields(world: &World, centre: [i64; 3], (level, side): (u32, i64), fields: &[&Field], found: &mut Found) -> Vec<(usize, f64)> {
    let mut randoms: Vec<Rng> = fields.iter().map(|field| Rng::new(hash_text(field.name, world.generator.seed))).collect();
    let s = 1i64 << level;
    let cube = world.within(Box3::around(centre, side / 2));
    let (h0, h1) = (cube.min[1], cube.max[1]);
    let side = side / s;
    let mut drawn = vec![(0, 0.0); fields.len()];
    for c in 0..side * side {
        // A site's column of the level, as generating it asks: at any height, for a field of sites.
        let [x, z] = [(c % side, centre[0]), (c / side, centre[2])].map(|(i, o)| (o.div_euclid(s) + i - side / 2) * s);
        for ((field, random), drawn) in fields.iter().zip(&mut randoms).zip(&mut drawn) {
            let start = Instant::now();
            let p = [x, h0 + random.below((h1 - h0) as u32) as i64, z];
            drawn.0 += survey_site(field, p, s, found) as usize;
            drawn.1 += start.elapsed().as_secs_f64();
        }
    }
    drawn
}

/** What `field` draws at `p`, a site of sites `s` apart, into `found`; whether anything. */
fn survey_site(field: &Field, p: [i64; 3], s: i64, found: &mut Found) -> bool {
    let [x, _, z] = p;
    // What it draws there, along an axis from..to: the place is around the top of what fills, the middle of what carves.
    let (kinds, along, from, to): (Vec<Kind>, usize, i64, i64) = match &field.values {
        FieldValues::At(at) => {
            let at = |s| footprint(s, [0, 0], || at(x, p[1], z));
            let Some((kind, _)) = at(s) else { return false };
            (at(1).into_iter().map(|(k, _)| k).chain([kind]).collect(), 1, p[1], p[1] + 1)
        }
        FieldValues::Shapes(near) => {
            // A shape reaching the site's voxel, from its bottom to its top.
            let b = Box3::around(p, s);
            let shapes = footprint(s, [0, 0], || near(&b));
            let Some(shape) = shapes.iter().find(|shape| shape.bounds.overlaps(&b)) else { return false };
            (vec![shape.kind], 1, shape.bounds.min[1], shape.bounds.max[1])
        }
        FieldValues::Spans { axis, spans } => {
            let [a, b] = across(*axis);
            let spans = |s| footprint(s, [0, 0], || spans(p[a], p[b]));
            let coarse = spans(s);
            if coarse.is_empty() {
                return false;
            }
            // And what it draws finer than these sites, at level 0, where the place is: a coarse
            // site's span is stretched to keep a site (column.rs), so its top may be far from the ground's.
            let fine = spans(1);
            let at = if fine.is_empty() { &coarse } else { &fine };
            let (from, to) = (at.iter().map(|s| s.from).min().expect("spans"), at.iter().map(|s| s.to).max().expect("spans"));
            (coarse.iter().chain(&fine).map(|s| s.kind).collect(), *axis, from, to)
        }
    };
    let carves = field.op == Op::Carve;
    let at = if carves { (from + to) / 2 } else { to };
    // The site, moved there: fields whose surfaces are near share a region.
    let mut centre = p;
    centre[along] = at;
    let place = Box3::around(centre, s / 2);
    let kinds: BTreeSet<Option<Kind>> = if carves { [None].into() } else { kinds.into_iter().map(Some).collect() };
    for kind in kinds {
        found.add((field.name.to_string(), kind), 1, [place]);
    }
    true
}

/** The axes across `axis`, in order. */
fn across(axis: usize) -> [usize; 2] {
    [(axis == 0) as usize, 2 - (axis == 2) as usize]
}

/**
 * The things of `population` in a cube around `centre`, from 16 cells
 * wide, twice as wide each time while they are taken apart into fewer
 * than PARTS / 4 parts, up to SIDE; their parts, into `found`. Returns
 * half the cube's side, and the things.
 */
fn survey_things(world: &World, centre: [i64; 3], population: &Population, region: i64, found: &mut Found) -> (i64, Things) {
    let (n, reach) = (population.cell, population.reach);
    let [ci, _, ck] = centre.map(|v| v.div_euclid(n));
    let (mut half, most) = (8, (SIDE / 2 / n).min(HALF_CELLS));
    loop {
        let (mut parts, mut count) = (Found::new(world.spawn), 0);
        let mut seen = Things { population: population.name, reach, made: 0, past: Vec::new(), leaving: Vec::new() };
        // Along y, the cells whose reach meets the cube within the heights.
        let cube = world.within(Box3::around(centre, half * n));
        let (j0, j1) = ((cube.min[1] - n - reach[1]).div_euclid(n), (cube.max[1] + n + reach[1]).div_euclid(n) + 1);
        let cells = [(ci - half, ci + half), (j0, j1), (ck - half, ck + half)];
        each_made(&world.generator, population, cells, |key, bounds, allowed, node| {
            seen.made += 1;
            if !bounds.inside(&allowed) {
                seen.past.push((key.clone(), bounds, allowed));
            }
            let leaves = world.heights.is_some_and(|(h0, h1)| bounds.min[1] < h0 || bounds.max[1] > h1);
            if leaves && seen.leaving.len() < LEAVING {
                seen.leaving.push((key, bounds));
            }
            count += take_apart(node, population.name.to_string(), region, &mut parts);
        });
        if count * 4 > PARTS || half >= most {
            for (key, (n, places)) in parts.strata {
                found.add(key, n, places);
            }
            return (half * n, seen);
        }
        half = (2 * half).min(most);
    }
}

/**
 * `node`, named `name`, into parts at most `region` wide: itself if it
 * is, else its own shapes and its children's parts, each named by the
 * names down to it; each a place of every kind it draws. Returns how
 * many parts.
 */
fn take_apart(node: &Node, name: String, region: i64, parts: &mut Found) -> usize {
    let bounds = node.bounds();
    if fits(&bounds, region) || node.children.is_none() {
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
    each_child(node, |child, node| count += take_apart(node, format!("{name}/{}", unnumbered(child)), region, parts));
    count
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

impl Survey {
    /**
     * Regions, each at most `region` wide, holding a place of every
     * stratum: the rarest first, each in the region it widens least,
     * or else its place nearest spawn starts one; of the strata
     * `wanted`, by index. With the strata each holds, by index.
     */
    pub fn regions(&self, wanted: impl Fn(&usize) -> bool) -> Vec<(Box3, Vec<usize>)> {
        let mut order: Vec<usize> = (0..self.strata.len()).filter(wanted).collect();
        order.sort_by_key(|&i| (self.strata[i].found, i));
        let mut regions: Vec<(Box3, Vec<usize>)> = Vec::new();
        let volume = |b: &Box3| (0..3).map(|i| (b.max[i] - b.min[i]) as f64).product::<f64>();
        for i in order {
            let mut best: Option<(f64, usize, Box3)> = None;
            for place in &self.strata[i].places {
                for (r, (region, _)) in regions.iter().enumerate() {
                    let union = Box3 {
                        min: [0, 1, 2].map(|a| region.min[a].min(place.min[a])),
                        max: [0, 1, 2].map(|a| region.max[a].max(place.max[a])),
                    };
                    let growth = volume(&union) - volume(region);
                    if fits(&union, self.region) && best.as_ref().is_none_or(|(g, ..)| growth < *g) {
                        best = Some((growth, r, union));
                    }
                }
            }
            match best {
                Some((_, r, union)) => {
                    regions[r].0 = union;
                    regions[r].1.push(i);
                }
                // Its places are nearest spawn first.
                None => regions.push((self.strata[i].places[0], vec![i])),
            }
        }
        regions
    }

    /** The places of stratum `i` that meet `b`. */
    pub fn places_in(&self, i: usize, b: &Box3) -> Vec<Box3> {
        self.strata[i].places.iter().filter(|p| p.overlaps(b)).copied().collect()
    }

    /** The kinds of `generator` that nothing surveyed draws, by name. */
    pub fn unseen(&self, generator: &Generator) -> Vec<&'static str> {
        let seen: BTreeSet<Kind> = self.strata.iter().filter_map(|s| s.kind).collect();
        generator.kinds.iter().enumerate().filter(|(i, _)| !seen.contains(&(*i as Kind))).map(|(_, k)| k.name).collect()
    }

    /** A stratum's name and kind, for messages. */
    pub fn describe(&self, generator: &Generator, i: usize) -> String {
        let s = &self.strata[i];
        format!("{} {}", s.name, s.kind.map_or("carving", |k| generator.kinds[k as usize].name))
    }
}
