//! The things populations make in the cells of N·FCC, each cell asked
//! alone: for `generate`, and for checks that look at more things than a
//! box draws.

use super::*;

/**
 * Each thing `population` makes in the cells of N·FCC whose indices are
 * in `cells` ([first, last] along each axis), with its cell: those whose
 * anchors are in them, asked of each cell or once of each line of them;
 * returns the work of asking.
 */
pub(super) fn each_thing(
    generator: &Generator,
    population: &Population,
    cells: [(i64, i64); 3],
    mut each: impl FnMut(&Cell, Point, Node),
) -> Work {
    let mut work = Work::default();
    let n = population.cell;
    let cell = |c: [i64; 3]| Cell::fcc(population, generator.seed, c);
    let mut make = |cell: &Cell, anchor: Point| {
        if let Some(ground) = &population.on {
            let [x, y, z] = anchor;
            let g = ground.at(x.round() as i64, z.round() as i64);
            assert!(
                (y - g as f64).abs() <= n as f64,
                "{} is anchored at {anchor:?}, {:.0} units off the ground it stands on (Population::on), more than its cell's width {n}; see \"Rules\" in {DOCS}",
                cell.key(),
                y - g as f64
            );
        }
        if let Some(node) = (population.make)(anchor, cell) {
            each(cell, anchor, node);
        }
    };
    match &population.anchor {
        Anchors::Cells(anchor) => {
            let [(i0, i1), (j0, j1), (k0, k1)] = cells;
            for i in i0..=i1 {
                for j in j0..=j1 {
                    for k in (k0 + ((k0 + i + j) & 1)..=k1).step_by(2) {
                        let cell = cell([i, j, k]);
                        work.cells += 1;
                        if let Some(p) = anchor(&cell).filter(|p| nearest_site(at(*p), n) == [i, j, k]) {
                            work.owned += 1;
                            make(&cell, p);
                        }
                    }
                }
            }
        }
        Anchors::Lines { axis, anchor } => {
            let (axis, [a, b]) = (*axis, across(*axis));
            for u in cells[a].0..=cells[a].1 {
                for v in cells[b].0..=cells[b].1 {
                    let line = Line { axis, across: [u, v], centre: [u * n, v * n], n, population: population.name, seed: generator.seed };
                    work.cells += 1;
                    let Some(p) = anchor(&line) else { continue };
                    // Owned by the cell of the line it is in, if the box reaches it.
                    let owner = nearest_site(at(p), n);
                    if owner[a] == u && owner[b] == v && (cells[axis].0..=cells[axis].1).contains(&owner[axis]) {
                        work.owned += 1;
                        make(&cell(owner), p);
                    }
                }
            }
        }
        Anchors::Points { points, anchor } => {
            // Cells are leaves along the axes the points span; along the others, those their anchors are nearest.
            let spans = points.spans();
            work.cells += (0..3).filter(|a| spans[*a]).map(|a| (cells[a].1 - cells[a].0 + 1) as u64).product::<u64>();
            for spot in points.points_in(cells.map(|c| c.0 * n), cells.map(|c| (c.1 + 1) * n)) {
                let Some(p) = anchor(&spot) else { continue };
                let along = |a: usize| (p[a].round() as i64 + n / 2).div_euclid(n);
                if (0..3).any(|a| !spans[a] && !(cells[a].0..=cells[a].1).contains(&along(a))) {
                    continue;
                }
                let [i, j, k] = [0, 1, 2].map(|a| if spans[a] { spot.leaf[a].div_euclid(n) } else { 0 });
                let centre = [0, 1, 2].map(|a| if spans[a] { spot.leaf[a] + n / 2 } else { p[a].round() as i64 });
                let (mark, nth) = (spot.mark, spot.nth);
                work.owned += 1;
                make(&Cell { i, j, k, centre, n: n / 2, mark, nth, population: population.name, seed: generator.seed }, p);
            }
        }
    }
    work
}

/** The box a thing of `population` in `cell` may reach: N and the population's reach from the cell's centre. */
pub(super) fn reach_of(population: &Population, cell: &Cell) -> Box3 {
    let (c, n, r) = (cell.centre, cell.n, population.reach);
    Box3 { min: [0, 1, 2].map(|i| c[i] - n - r[i]), max: [0, 1, 2].map(|i| c[i] + n + r[i]) }
}

/**
 * The things `population` makes in the cells of N·FCC whose indices are
 * in `cells` ([first, last] along each axis), as `generate` makes them
 * at level 0 (coarser, they stand on the ground as their level draws it):
 * each one's key, its bounds and the box its population's reach allows
 * it. For checks that look at more things than a box draws
 * (worldgen::checks::contract).
 */
pub fn things_in(generator: &Generator, population: &Population, cells: [(i64, i64); 3]) -> Vec<(String, Box3, Box3)> {
    let mut things = Vec::new();
    each_made(generator, population, cells, |key, bounds, allowed, _| things.push((key, bounds, allowed)));
    things
}

/** As `things_in`, each with the thing itself, to look into (worldgen::checks::survey); returns how many cells were asked. */
pub fn each_made(
    generator: &Generator,
    population: &Population,
    cells: [(i64, i64); 3],
    mut each: impl FnMut(String, Box3, Box3, &Node),
) -> u64 {
    each_thing(generator, population, cells, |cell, _, node| each(cell.key(), bounds_of(&node), reach_of(population, cell), &node)).cells
}

/**
 * The anchor of the thing `population` makes in cell (i, j, k) of N·FCC
 * (i + j + k even), and the thing, as `generate` makes them at level 0; none if it
 * makes none there.
 */
pub fn made_in(generator: &Generator, population: &Population, i: i64, j: i64, k: i64) -> Option<(Point, Node)> {
    let (anchor, owner) = anchor_in(generator, population, i, j, k)?;
    let cell = Cell::fcc(population, generator.seed, [i, j, k]);
    (owner == [i, j, k]).then(|| (population.make)(anchor, &cell)).flatten().map(|node| (anchor, node))
}

/**
 * The anchors of the things `population` makes in the cells whose
 * indices are in `cells` ([first, last] along each axis), as `generate`
 * makes them: anchored at points, those of the points of these leaves.
 */
pub fn anchors_in(generator: &Generator, population: &Population, cells: [(i64, i64); 3]) -> Vec<Point> {
    let mut anchors = Vec::new();
    each_thing(generator, population, cells, |_, anchor, _| anchors.push(anchor));
    anchors
}

/**
 * The anchor asked for cell (i, j, k) of `population`, or for its line,
 * if any, and the cell it is in, which owns a thing there if it is this
 * one; none anchored at points.
 */
pub fn anchor_in(generator: &Generator, population: &Population, i: i64, j: i64, k: i64) -> Option<(Point, [i64; 3])> {
    let n = population.cell;
    let anchor = match &population.anchor {
        Anchors::Cells(anchor) => anchor(&Cell::fcc(population, generator.seed, [i, j, k])),
        Anchors::Lines { axis, anchor } => {
            let ([a, b], c) = (across(*axis), [i, j, k]);
            anchor(&Line {
                axis: *axis,
                across: [c[a], c[b]],
                centre: [c[a] * n, c[b] * n],
                n,
                population: population.name,
                seed: generator.seed,
            })
        }
        // A leaf holds any number: see anchors_in.
        Anchors::Points { .. } => None,
    }?;
    Some((anchor, nearest_site(at(anchor), n)))
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
