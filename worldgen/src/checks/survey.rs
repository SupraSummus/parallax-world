//! What a world is made of, found by looking rather than listed: every
//! layer of its generator asked over a square around spawn (or, drawing
//! nothing there, around each place the world names), things taken
//! apart into parts no wider than a region, sorted into strata: a part or
//! a field, with a kind it draws. The contract checks a place of every
//! stratum found (worldgen::checks::contract), however rare, without a list.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::World;
use crate::generator::{Children, Field, FieldValues, Generator, Kind, Layer, Node, Op, Population, Shape, bounds_of_shapes, each_made};
use crate::grid::Box3;
use crate::noise::{Rng, hash_text};
use crate::units::m;

/** The side of the square around spawn layers are surveyed over (16 km), lattice units. */
const SIDE: i64 = m(16_000.0);

/**
 * Parts a population's things are taken apart into, about, at most:
 * past it, the square it is surveyed over stops growing.
 */
const PARTS: usize = 20_000;

/**
 * The level whose columns of sites each field is asked for, over the
 * square: sites 16 m apart. Fields keep what is finer than the sites in
 * proportion ("Plans" in docs/generators.md), so a narrow one shows.
 */
const FIELD_LEVEL: u32 = 7;

/**
 * Around a place the world names, a field is asked at the columns of a
 * finer level over a smaller square: sites a metre apart, 128 m. Where the
 * world says a feature is, one narrower than FIELD_LEVEL's sites that it
 * doesn't keep far away (a tunnel's bore) is found there, at any seed.
 */
const PLACE_LEVEL: u32 = 3;
const PLACE_SIDE: i64 = m(128.0);

/** Things whose bounds leave the heights that the survey keeps, at most, of each population. */
const LEAVING: usize = 16;

/** A kind a part of things or a field draws, or none if it carves: a stratum, and its places. */
pub struct Stratum {
    /** The layer, and in a thing the names of its parts down to this one, numbers left out. */
    pub name: String,
    /** By index in Generator::kinds. */
    pub kind: Option<Kind>,
    /** A box around each, at most a region wide: parts' bounds, around a field's samples. */
    pub places: Vec<Box3>,
}

/** How far a layer was surveyed. */
pub struct Surveyed {
    pub layer: &'static str,
    /** Half the side of the square around spawn, lattice units. */
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

type Found = BTreeMap<(String, Option<Kind>), Vec<Box3>>;

/** What `world` is made of around spawn, in places at most `region` wide (lattice units): of the layers `wanted` by name. */
pub fn survey(world: &World, region: i64, wanted: impl Fn(&str) -> bool) -> Survey {
    let mut found = Found::new();
    let (mut layers, mut things) = (Vec::new(), Vec::new());
    for layer in &world.generator.layers {
        let layer_name = match layer {
            Layer::Population(p) => p.name,
            Layer::Field(f) => f.name,
        };
        if !wanted(layer_name) {
            continue;
        }
        let start = Instant::now();
        match layer {
            Layer::Population(population) => {
                let (half, seen) = around(world, |centre, _| survey_things(world, centre, population, region, &mut found), |s| s.1.made);
                layers.push(Surveyed { layer: population.name, half, found: seen.made, seconds: start.elapsed().as_secs_f64() });
                things.push(seen);
            }
            Layer::Field(field) => {
                let drawn = around(world, |centre, square| survey_field(world, centre, square, field, region, &mut found), |d| *d);
                layers.push(Surveyed { layer: field.name, half: SIDE / 2, found: drawn, seconds: start.elapsed().as_secs_f64() });
            }
        }
    }
    let strata = found.into_iter().map(|((name, kind), places)| Stratum { name, kind, places }).collect();
    Survey { spawn: world.spawn, region, strata, layers, things }
}

/**
 * `survey` around spawn over the whole square, or, finding nothing there
 * (`found` is 0), around each place the world names in turn, finer:
 * a feature too rare to be near spawn at every seed (the valley's
 * tunnels) is where the world says it is.
 */
fn around<T>(world: &World, mut survey: impl FnMut([i64; 3], (u32, i64)) -> T, found: impl Fn(&T) -> usize) -> T {
    let mut done = survey(world.spawn, (FIELD_LEVEL, SIDE));
    for (_, find) in &world.points {
        if found(&done) > 0 {
            break;
        }
        done = survey(find(), (PLACE_LEVEL, PLACE_SIDE));
    }
    done
}

/**
 * Where `field` draws around `centre`, asked at every column of sites of
 * `level` over a square `side` wide, into `found`; returns at how many.
 */
fn survey_field(world: &World, centre: [i64; 3], (level, side): (u32, i64), field: &Field, region: i64, found: &mut Found) -> usize {
    let mut random = Rng::new(hash_text(field.name, world.generator.seed));
    let ((h0, h1), half, s) = (world.heights, region / 2, 1i64 << level);
    let side = side / s;
    let mut drawn = 0;
    for c in 0..side * side {
        // A site's column of FIELD_LEVEL, as generating it asks: at any height, for a field of sites.
        let [x, z] = [(c % side, centre[0]), (c / side, centre[2])].map(|(i, o)| (o.div_euclid(s) + i - side / 2) * s);
        let p = [x, h0 + random.below((h1 - h0) as u32) as i64, z];
        // What it draws there, along an axis from..to: the place is around the top of what fills, the middle of what carves.
        let (kinds, along, from, to): (Vec<Kind>, usize, i64, i64) = match &field.values {
            FieldValues::At(at) => match at(x, p[1], z, s) {
                Some((kind, _)) => (at(x, p[1], z, 1).into_iter().map(|(k, _)| k).chain([kind]).collect(), 1, p[1], p[1] + 1),
                None => continue,
            },
            FieldValues::Spans { axis, spans } => {
                let [a, b] = across(*axis);
                let coarse = spans(p[a], p[b], s);
                if coarse.is_empty() {
                    continue;
                }
                // And what it draws finer than these sites, at level 0, where the place is: a coarse
                // site's span is stretched to keep a site (column.rs), so its top may be far from the ground's.
                let fine = spans(p[a], p[b], 1);
                let at = if fine.is_empty() { &coarse } else { &fine };
                let (from, to) = (at.iter().map(|s| s.from).min().expect("spans"), at.iter().map(|s| s.to).max().expect("spans"));
                (coarse.iter().chain(&fine).map(|s| s.kind).collect(), *axis, from, to)
            }
        };
        drawn += 1;
        let carves = field.op == Op::Carve;
        let at = if carves { (from + to) / 2 } else { to };
        let mut place = Box3 { min: p.map(|v| v - s / 2), max: p.map(|v| v + s / 2) };
        (place.min[along], place.max[along]) = (at - half, at + half);
        let kinds: BTreeSet<Option<Kind>> = if carves { [None].into() } else { kinds.into_iter().map(Some).collect() };
        for kind in kinds {
            found.entry((field.name.to_string(), kind)).or_default().push(place);
        }
    }
    drawn
}

/** The axes across `axis`, in order. */
fn across(axis: usize) -> [usize; 2] {
    [(axis == 0) as usize, 2 - (axis == 2) as usize]
}

/**
 * The things of `population` in a square around `centre`, from 16 cells
 * wide, twice as wide each time while they are taken apart into fewer
 * than PARTS / 4 parts, up to SIDE; their parts, into `found`. Returns
 * half the square's side, and the things.
 */
fn survey_things(world: &World, centre: [i64; 3], population: &Population, region: i64, found: &mut Found) -> (i64, Things) {
    let (n, reach, (h0, h1)) = (population.cell, population.reach, world.heights);
    let [ci, _, ck] = centre.map(|v| v.div_euclid(n));
    // Cells whose reach meets the heights.
    let (j0, j1) = ((h0 - n - reach[1]).div_euclid(n), (h1 + n + reach[1]).div_euclid(n) + 1);
    let mut half = 8;
    loop {
        let (mut parts, mut count) = (Found::new(), 0);
        let mut seen = Things { population: population.name, reach, made: 0, past: Vec::new(), leaving: Vec::new() };
        let cells = [(ci - half, ci + half), (j0, j1), (ck - half, ck + half)];
        each_made(&world.generator, population, cells, |key, bounds, allowed, node| {
            seen.made += 1;
            if !bounds.inside(&allowed) {
                seen.past.push((key.clone(), bounds, allowed));
            }
            if (bounds.min[1] < h0 || bounds.max[1] > h1) && seen.leaving.len() < LEAVING {
                seen.leaving.push((key, bounds));
            }
            count += take_apart(node, population.name.to_string(), region, &mut parts);
        });
        if count * 4 > PARTS || half >= SIDE / 2 / n {
            for (key, mut places) in parts {
                found.entry(key).or_default().append(&mut places);
            }
            return (half * n, seen);
        }
        half = (2 * half).min(SIDE / 2 / n);
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
        kinds.into_iter().for_each(|kind| parts.entry((name.clone(), Some(kind))).or_default().push(place));
        return 1;
    }
    let own = node.shapes.get();
    let mut count = 0;
    if own.iter().any(|s| s.op != Op::Carve) {
        let place = centred(&bounds_of_shapes(own.iter().map(|s| &s.bounds)), region);
        let kinds: BTreeSet<Kind> = shapes_kinds(own).collect();
        kinds.into_iter().for_each(|kind| parts.entry((format!("{name} itself"), Some(kind))).or_default().push(place));
        count += 1;
    }
    each_child(node, |child, node| count += take_apart(node, format!("{name} › {}", unnumbered(child)), region, parts));
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

/** A part's name without its numbers: "block 3 1 2" is a block. */
fn unnumbered(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().filter(|w| !w.chars().any(|c| c.is_ascii_digit())).collect();
    if words.is_empty() { "?".into() } else { words.join(" ") }
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
        order.sort_by_key(|&i| (self.strata[i].places.len(), i));
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
                None => {
                    let distance = |b: &&Box3| [0, 2].map(|a| ((b.min[a] + b.max[a]) / 2 - self.spawn[a]).pow(2)).iter().sum::<i64>();
                    let nearest = self.strata[i].places.iter().min_by_key(distance).expect("a stratum has places");
                    regions.push((*nearest, vec![i]));
                }
            }
        }
        regions
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
