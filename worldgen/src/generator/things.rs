//! The things populations make at their points, each made alone: for
//! `generate`, and for checks that look at more things than a box draws.

use super::*;

/**
 * Each thing `population` makes anchored in box `b` (lattice units),
 * with its anchor; returns the work of asking.
 */
pub(super) fn each_thing(generator: &Generator, population: &Population, b: &Box3, mut each: impl FnMut(&Anchor, Node)) -> Work {
    let mut work = Work::default();
    // How many of its things are anchored at each point so far: the next one there is known by its number.
    let mut at: HashMap<[i64; 3], u32> = HashMap::new();
    (population.things)(b, &mut |p, make| {
        work.points += 1;
        let key = p.map(|v| v.round() as i64);
        if !b.holds(key) {
            return;
        }
        work.inside += 1;
        let nth = at.entry(key).or_insert(0);
        let anchor = Anchor { at: p, key, nth: *nth, population: population.name, seed: generator.seed };
        *nth += 1;
        let Some(node) = make(&anchor) else { return };
        if let Some(stands) = &population.stands {
            assert!(
                stands(&Box3 { min: key, max: key.map(|v| v + 1) }),
                "{} is anchored where its population says none stand (Population::stands); see \"Rules\" in {DOCS}",
                anchor.name()
            );
        }
        each(&anchor, node);
    });
    work
}

/** The box a thing of `population` anchored in it may reach `b` from (lattice units): `b` and its reach about it. */
pub(super) fn anchors_reaching(population: &Population, b: &Box3) -> Box3 {
    let r = population.reach;
    Box3 { min: [0, 1, 2].map(|a| b.min[a] - r[a]), max: [0, 1, 2].map(|a| b.max[a] + r[a]) }
}

/**
 * The things `population` makes that may reach box `b` (lattice units),
 * as `generate` makes them at level 0 (coarser, they stand on the ground
 * as their level draws it): each one's name, its bounds and the box its
 * population's reach allows it. For checks that look at more things than
 * a box draws (worldgen::checks::contract).
 */
pub fn things_in(generator: &Generator, population: &Population, b: &Box3) -> Vec<(String, Box3, Box3)> {
    let mut things = Vec::new();
    each_made(generator, population, b, |name, bounds, allowed, _| things.push((name, bounds, allowed)));
    things
}

/** As `things_in`, each with the thing itself, to look into (worldgen::checks::survey); returns how many points were asked. */
pub fn each_made(generator: &Generator, population: &Population, b: &Box3, mut each: impl FnMut(String, Box3, Box3, &Node)) -> u64 {
    let reach = population.reach;
    each_thing(generator, population, &anchors_reaching(population, b), |anchor, node| {
        each(anchor.name(), bounds_of(&node), anchor.reach(reach), &node)
    })
    .points
}

/** The anchors of the things `population` makes anchored in box `b` (lattice units), as `generate` makes them. */
pub fn anchors_in(generator: &Generator, population: &Population, b: &Box3) -> Vec<Point> {
    let mut anchors = Vec::new();
    each_thing(generator, population, b, |anchor, _| anchors.push(anchor.at));
    anchors
}

pub(super) fn bounds_of(node: &Node) -> Box3 {
    if let Some(bounds) = node.bounds {
        return bounds;
    }
    assert!(node.children.is_none(), "a node with children needs bounds; see \"Things\" in {DOCS}");
    bounds_of_shapes(node.shapes.get().iter().map(|s| &s.bounds))
}

/** The box holding the boxes. */
pub fn bounds_of_shapes<'a>(boxes: impl IntoIterator<Item = &'a Box3>) -> Box3 {
    let mut b = Box3 { min: EVERYWHERE.max, max: EVERYWHERE.min };
    for s in boxes {
        for i in 0..3 {
            b.min[i] = b.min[i].min(s.min[i]);
            b.max[i] = b.max[i].max(s.max[i]);
        }
    }
    b
}
