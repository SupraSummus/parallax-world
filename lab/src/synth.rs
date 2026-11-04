//! Sampling a world top down from learned rule-sets, any box alone.
//!
//! Each level refines a cell of its parent level into b × b children, by
//! WFC (wfc.rs) with the rule-set of the parent's label. The children are
//! solved in three rounds, each part from its own position and parents
//! only, so a box is the same whatever else is generated:
//!
//! - corners: the 2 × 2 children around each corner of the parents' grid,
//!   from the four parents' labels;
//! - edges: the 2 × (b − 2) children along each edge, from its two parents
//!   and the corners at its ends;
//! - insides: the (b − 2) × (b − 2) children of each parent, from it and its
//!   ring of edges and corners.
//!
//! A rule-set is what its biome's blocks showed: which pairs of
//! neighbours occur (the hard constraints: inside one parent, its own;
//! across two parents, the transitions seen across blocks of their two
//! biomes, and what both their rule-sets allow), the labels' shares, and
//! either the chance of each label in each context of neighbours or (for
//! comparison) the pairs' mutual information alone. A part that
//! contradicts is retried with new seeds, then drawn without the hard
//! constraints. The top level refines a single root label, everywhere.

use worldgen::noise::{hash64, stream};

use crate::image::Image;
use crate::wfc::{self, Adjacency, Cell, Contexts, DIRECTIONS, Link, MAX_LABELS, Other};

/** How a level refines parent labels into children. */
pub struct Rules {
    pub b: usize,
    /** Child labels, parent labels. */
    pub n: usize,
    pub parents: usize,
    /** Per parent label: log share of each child label (−∞ if none). */
    pub prior: Vec<Vec<f32>>,
    /** Per parent label, along x and y: pairs inside one parent. */
    pub inside: Vec<[Adjacency; 2]>,
    /** Per pair of parent labels (p * parents + q, p left of or below q), along x and y: pairs across two parents. */
    pub across: Vec<[Adjacency; 2]>,
    /** Per parent label: its children's labels in each context of neighbours, to weigh labels by (or pairs alone). */
    pub contexts: Option<Vec<Contexts>>,
}

/** A pair seen fewer times than this in a rule-set is not allowed. */
const MIN_PAIRS: f64 = 2.0;
/** Pseudo-counts of the two rule-sets added to the transitions across two parents. */
const ACROSS_PRIOR: f64 = 20.0;

/** The rules refining `parents` into `children` (b × b children per parent), counted from both; weighing by `contexts` or pairs. */
pub fn rules(children: &Image, parents: &Image, b: usize, contexts: bool) -> Rules {
    assert!(b >= 3, "blocks of at least 3: corners, edges and insides");
    let (n, np) = (children.labels(), parents.labels());
    assert!(n <= MAX_LABELS, "{n} labels");
    let mut unary = vec![vec![0.0; n]; np];
    let mut inside = vec![[vec![0.0; n * n], vec![0.0; n * n]]; np];
    let mut across = vec![[vec![0.0; n * n], vec![0.0; n * n]]; np * np];
    let mut seen = contexts.then(|| (0..np).map(|_| Contexts::new(n)).collect::<Vec<_>>());
    let (w, h) = (children.w.min(parents.w * b), children.h.min(parents.h * b));
    for y in 0..h {
        for x in 0..w {
            let (a, p) = (children.at(x, y) as usize, parents.at(x / b, y / b) as usize);
            unary[p][a] += 1.0;
            if let Some(seen) = seen.as_mut() {
                let (mut mask, mut labels) = (0, [0u16; 4]);
                for (side, (dx, dy)) in DIRECTIONS.into_iter().enumerate() {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < children.w && (ny as usize) < children.h {
                        mask |= 1 << side;
                        labels[side] = children.at(nx as usize, ny as usize);
                    }
                }
                seen[p].add(mask, &labels, a as u16);
            }
            for (dir, (nx, ny)) in [(x + 1, y), (x, y + 1)].into_iter().enumerate() {
                if nx >= w || ny >= h {
                    continue;
                }
                let (c, q) = (children.at(nx, ny) as usize, parents.at(nx / b, ny / b) as usize);
                if (nx / b, ny / b) == (x / b, y / b) {
                    inside[p][dir][a * n + c] += 1.0;
                } else {
                    across[p * np + q][dir][a * n + c] += 1.0;
                }
            }
        }
    }
    let prior = unary
        .iter()
        .map(|u| {
            let total: f64 = u.iter().sum::<f64>().max(1.0);
            u.iter().map(|c| if *c > 0.0 { (c / total).ln() as f32 } else { f32::NEG_INFINITY }).collect()
        })
        .collect();
    let share = |counts: &[f64]| {
        let total = counts.iter().sum::<f64>().max(1.0);
        counts.iter().map(|c| c / total).collect::<Vec<f64>>()
    };
    let across = (0..np * np)
        .map(|pq| {
            let (p, q) = (pq / np, pq % np);
            [0, 1].map(|dir| {
                let (sp, sq) = (share(&inside[p][dir]), share(&inside[q][dir]));
                let seen = &across[pq][dir];
                let counts: Vec<f64> = (0..n * n).map(|i| seen[i] + ACROSS_PRIOR * 0.5 * (sp[i] + sq[i])).collect();
                let (ip, iq) = (&inside[p][dir], &inside[q][dir]);
                Adjacency::new(n, &counts, &|a, c| {
                    let i = a * n + c;
                    seen[i] >= MIN_PAIRS || (ip[i] >= MIN_PAIRS && iq[i] >= MIN_PAIRS)
                })
            })
        })
        .collect();
    let inside = inside.iter().map(|d| [0, 1].map(|dir| Adjacency::new(n, &d[dir], &|a, c| d[dir][a * n + c] >= MIN_PAIRS))).collect();
    Rules { b, n, parents: np, prior, inside, across, contexts: seen }
}

/** Rules for each level, finest first: each refines the next's labels; the last refines the root. */
pub struct Model {
    pub levels: Vec<Rules>,
}

impl Model {
    /**
     * The model of `base` and its biome images `levels` (each with the block
     * size that made it), finest first, under a root refined in blocks of `root`,
     * weighing labels by `contexts` or by pairs alone.
     * With no levels, a single rule-set for `base`: flat WFC.
     */
    pub fn new(base: &Image, levels: &[(usize, &Image)], root: usize, contexts: bool) -> Model {
        let mut rules_ = Vec::new();
        let mut below = base;
        for (b, image) in levels {
            rules_.push(rules(below, image, *b, contexts));
            below = image;
        }
        let top = Image::filled(below.w.div_ceil(root), below.h.div_ceil(root), 0);
        rules_.push(rules(below, &top, root, contexts));
        Model { levels: rules_ }
    }
}

/** What sampling took: parts solved, retried and drawn without the hard constraints, per level. */
#[derive(Clone, Default, Debug)]
pub struct Report {
    pub parts: Vec<usize>,
    pub retried: Vec<usize>,
    pub fallbacks: Vec<usize>,
}

const RETRIES: u64 = 8;

/** A box of cells of a level: from (x0, y0), w × h. */
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub x0: i64,
    pub y0: i64,
    pub w: usize,
    pub h: usize,
}

impl Window {
    pub fn new(x0: i64, y0: i64, w: usize, h: usize) -> Window {
        Window { x0, y0, w, h }
    }
}

/** The labels of level `k` (0 the finest) over `window`, alone. */
pub fn sample(model: &Model, seed: u64, k: usize, window: Window, report: &mut Report) -> Image {
    let Window { x0, y0, w, h } = window;
    if k == model.levels.len() {
        return Image::filled(w, h, 0);
    }
    for v in [&mut report.parts, &mut report.retried, &mut report.fallbacks] {
        v.resize(model.levels.len(), 0);
    }
    let rules = &model.levels[k];
    let b = rules.b as i64;
    // The parents the box touches, and the ring around them that its corners and edges read.
    let (pi0, pj0) = (x0.div_euclid(b), y0.div_euclid(b));
    let (pi1, pj1) = ((x0 + w as i64 - 1).div_euclid(b) + 1, (y0 + h as i64 - 1).div_euclid(b) + 1);
    let parents = sample(model, seed, k + 1, Window::new(pi0 - 1, pj0 - 1, (pi1 - pi0 + 2) as usize, (pj1 - pj0 + 2) as usize), report);
    let parent = |cx: i64, cy: i64| parents.at((cx.div_euclid(b) - pi0 + 1) as usize, (cy.div_euclid(b) - pj0 + 1) as usize) as usize;
    // Children of those parents and one more around, where the corners and edges reach.
    let (cx0, cy0) = (pi0 * b - 1, pj0 * b - 1);
    let (cw, ch) = (((pi1 - pi0) * b + 2) as usize, ((pj1 - pj0) * b + 2) as usize);
    let mut canvas = Image::filled(cw, ch, u16::MAX);
    let seed = stream(seed, k as u64);
    let mut solve = |part: u64, i: i64, j: i64, ex: [i64; 2], ey: [i64; 2], canvas: &mut Image| {
        let (ew, eh) = ((ex[1] - ex[0]) as usize, (ey[1] - ey[0]) as usize);
        let decided = |canvas: &Image, x: i64, y: i64| {
            let (u, v) = (x - cx0, y - cy0);
            (u >= 0 && v >= 0 && (u as usize) < cw && (v as usize) < ch)
                .then(|| canvas.at(u as usize, v as usize))
                .filter(|l| *l != u16::MAX)
        };
        let mut cells = Vec::with_capacity(ew * eh);
        for y in ey[0]..ey[1] {
            for x in ex[0]..ex[1] {
                let p = parent(x, y);
                let mut links = Vec::new();
                for (side, (dx, dy)) in DIRECTIONS.into_iter().enumerate() {
                    let (nx, ny, dir, first) = (x + dx, y + dy, side % 2, side < 2);
                    let other = if (ex[0]..ex[1]).contains(&nx) && (ey[0]..ey[1]).contains(&ny) {
                        Other::Cell(((ny - ey[0]) as usize) * ew + (nx - ex[0]) as usize)
                    } else if let Some(l) = decided(canvas, nx, ny) {
                        Other::Fixed(l)
                    } else {
                        continue;
                    };
                    let q = parent(nx, ny);
                    let same = (nx.div_euclid(b), ny.div_euclid(b)) == (x.div_euclid(b), y.div_euclid(b));
                    let adjacency = if same {
                        &rules.inside[p][dir]
                    } else if first {
                        &rules.across[p * rules.parents + q][dir]
                    } else {
                        &rules.across[q * rules.parents + p][dir]
                    };
                    links.push(Link { other, side, adjacency, first });
                }
                cells.push(Cell { prior: &rules.prior[p], links, contexts: rules.contexts.as_ref().map(|c| &c[p]) });
            }
        }
        report.parts[k] += 1;
        let at = |attempt: u64| hash64(hash64(seed, part as i64, attempt as i64, 0), i, j, 0);
        let labels = (0..RETRIES).find_map(|attempt| wfc::solve(&cells, at(attempt), true).map(|l| (attempt, l)));
        let labels = match labels {
            Some((attempt, labels)) => {
                report.retried[k] += (attempt > 0) as usize;
                labels
            }
            None => {
                report.fallbacks[k] += 1;
                wfc::solve(&cells, at(RETRIES), false).expect("unconstrained WFC never contradicts")
            }
        };
        for (n, l) in labels.into_iter().enumerate() {
            let (x, y) = (ex[0] + (n % ew) as i64, ey[0] + (n / ew) as i64);
            canvas.set((x - cx0) as usize, (y - cy0) as usize, l);
        }
    };
    for j in pj0..=pj1 {
        for i in pi0..=pi1 {
            solve(0, i, j, [i * b - 1, i * b + 1], [j * b - 1, j * b + 1], &mut canvas);
        }
    }
    for j in pj0..=pj1 {
        for i in pi0..=pi1 {
            if j < pj1 {
                solve(1, i, j, [i * b - 1, i * b + 1], [j * b + 1, j * b + b - 1], &mut canvas);
            }
            if i < pi1 {
                solve(2, i, j, [i * b + 1, i * b + b - 1], [j * b - 1, j * b + 1], &mut canvas);
            }
        }
    }
    for j in pj0..pj1 {
        for i in pi0..pi1 {
            solve(3, i, j, [i * b + 1, i * b + b - 1], [j * b + 1, j * b + b - 1], &mut canvas);
        }
    }
    canvas.crop((x0 - cx0) as usize, (y0 - cy0) as usize, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Two biomes at random in blocks of 8: one of labels 0 and 1, one of label 2 sprinkled with 1. */
    fn model() -> (Model, Image) {
        let (w, h, b) = (64, 64, 8);
        let biomes = Image::new(w / b, h / b, (0..(w / b) * (h / b)).map(|i| (hash64(1, i as i64, 0, 0) & 1) as u16).collect());
        let label = |x: usize, y: usize| match biomes.at(x / b, y / b) {
            0 => (hash64(2, x as i64, y as i64, 0) & 1) as u16,
            _ => 2 - hash64(2, x as i64, y as i64, 0).is_multiple_of(5) as u16,
        };
        let base = Image::new(w, h, (0..w * h).map(|i| label(i % w, i / w)).collect());
        (Model::new(&base, &[(b, &biomes)], 4, true), base)
    }

    #[test]
    fn a_box_alone_is_the_same_box_of_the_whole() {
        let (model, _) = model();
        let whole = sample(&model, 7, 0, Window::new(-40, -24, 120, 100), &mut Report::default());
        for (x, y, w, h) in [(0, 0, 13, 9), (37, 51, 40, 7), (-40, -24, 1, 1), (11, 3, 64, 64)] {
            let alone = sample(&model, 7, 0, Window::new(x, y, w, h), &mut Report::default());
            assert_eq!(alone, whole.crop((x + 40) as usize, (y + 24) as usize, w, h), "box {x} {y} {w} {h}");
        }
    }

    #[test]
    fn children_follow_their_parents_rule_sets() {
        let (model, _) = model();
        let mut report = Report::default();
        let biomes = sample(&model, 3, 1, Window::new(0, 0, 8, 8), &mut report);
        let image = sample(&model, 3, 0, Window::new(0, 0, 64, 64), &mut report);
        assert_eq!(report.fallbacks.iter().sum::<usize>(), 0, "{report:?}");
        for (i, l) in image.data.iter().enumerate() {
            let biome = biomes.at(i % 64 / 8, i / 64 / 8);
            assert!(if biome == 0 { *l < 2 } else { *l > 0 }, "label {l} in biome {biome}");
        }
    }
}
