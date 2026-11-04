//! Pixel conditionals with position: a quadtree of templates.
//!
//! Each coarser level's label is a template: a cluster of the b × b blocks
//! of the level below, a categorical per position in the block (a mixture
//! of products, by hard EM), so a coarse label can say "a street along the
//! block's west edge, two cells wide", which W0's biomes (bags of
//! neighbour pairs) could not.
//!
//! A level is refined into children cell by cell: a child is drawn from a
//! count table of what was seen given its parent, its position in the
//! parent, the parents across the edges it lies on, and its neighbours
//! decided before it, backing off (Witten–Bell) to fewer of them. The
//! order is W0's (synth.rs): the 2 × 2 children around each corner of the
//! parents' grid first, then the edges between, then the insides, raster
//! order within each part. So what was decided before a child is a
//! function of its position alone (it is in training too), and a box is
//! the same alone or as part of the whole.
//!
//! The top level is drawn from a single root over a whole crop, in raster
//! order, by position in the crop: where a town is is given, as the
//! valley's plans give it.

use std::collections::HashMap;

use worldgen::noise::{Rng, hash64};

use crate::image::Image;

/** Labels above the real ones: a parent beyond the crop, a neighbour not yet decided, no parent across. */
pub const OUTSIDE: u16 = 61;
const UNKNOWN: u16 = 62;
const NONE: u16 = 63;

/** Templates: per template, per position in the block, log P(child label). */
pub struct Templates {
    pub k: usize,
    pub n: usize,
    pub b: usize,
    pub logp: Vec<f32>,
}

impl Templates {
    fn score(&self, t: usize, block: &[u16]) -> f32 {
        let bb = self.b * self.b;
        block.iter().enumerate().map(|(p, l)| self.logp[(t * bb + p) * self.n + *l as usize]).sum()
    }

    fn best(&self, block: &[u16]) -> (usize, f32) {
        (0..self.k).map(|t| (t, self.score(t, block))).fold((0, f32::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a })
    }

    /** `image`'s whole blocks, each as its likeliest template. */
    pub fn coarsen(&self, image: &Image) -> Image {
        let b = self.b;
        let (w, h) = (image.w / b, image.h / b);
        let mut out = Image::filled(w, h, 0);
        let mut block = vec![0u16; b * b];
        for y in 0..h {
            for x in 0..w {
                for (p, l) in block.iter_mut().enumerate() {
                    *l = image.at(x * b + p % b, y * b + p / b);
                }
                out.set(x, y, self.best(&block).0 as u16);
            }
        }
        out
    }
}

/** The b × b blocks of `images`, distinct, with their counts, in a fixed order. */
fn distinct_blocks(images: &[Image], b: usize) -> Vec<(Vec<u16>, f32)> {
    let mut count: HashMap<Vec<u16>, u32> = HashMap::new();
    for image in images {
        for y in 0..image.h / b {
            for x in 0..image.w / b {
                let block: Vec<u16> = (0..b * b).map(|p| image.at(x * b + p % b, y * b + p / b)).collect();
                *count.entry(block).or_default() += 1;
            }
        }
    }
    let mut blocks: Vec<(Vec<u16>, f32)> = count.into_iter().map(|(b, c)| (b, c as f32)).collect();
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    blocks
}

/** `k` templates of the b × b blocks of `images` (labels below `n`), by hard EM from k-means++. */
pub fn fit(images: &[Image], n: usize, b: usize, k: usize, seed: u64) -> Templates {
    let blocks = distinct_blocks(images, b);
    let bb = b * b;
    let mut rng = Rng::new(seed);
    // k-means++ by Hamming distance, from the commonest block.
    let mut centres: Vec<usize> = vec![(0..blocks.len()).max_by(|a, c| blocks[*a].1.total_cmp(&blocks[*c].1)).unwrap_or(0)];
    let hamming = |a: &[u16], c: &[u16]| a.iter().zip(c).filter(|(x, y)| x != y).count() as f64;
    let mut nearest: Vec<f64> = blocks.iter().map(|bl| hamming(&bl.0, &blocks[centres[0]].0)).collect();
    while centres.len() < k.min(blocks.len()) {
        let total: f64 = nearest.iter().zip(&blocks).map(|(d, bl)| d * d * bl.1 as f64).sum();
        if total <= 0.0 {
            break;
        }
        let mut u = rng.next() * total;
        let mut pick = 0;
        for (i, (d, bl)) in nearest.iter().zip(&blocks).enumerate() {
            u -= d * d * bl.1 as f64;
            if u <= 0.0 {
                pick = i;
                break;
            }
        }
        centres.push(pick);
        for (i, bl) in blocks.iter().enumerate() {
            nearest[i] = nearest[i].min(hamming(&bl.0, &blocks[pick].0));
        }
    }
    let k = centres.len();
    let mut assign: Vec<usize> =
        blocks.iter().map(|bl| (0..k).min_by_key(|c| hamming(&bl.0, &blocks[centres[*c]].0) as u32).unwrap_or(0)).collect();
    let mut templates = Templates { k, n, b, logp: vec![0.0; k * bb * n] };
    for _ in 0..20 {
        let mut counts = vec![0.05f32; k * bb * n];
        for (bl, a) in blocks.iter().zip(&assign) {
            for (p, l) in bl.0.iter().enumerate() {
                counts[(a * bb + p) * n + *l as usize] += bl.1;
            }
        }
        for row in counts.chunks_mut(n) {
            let total: f32 = row.iter().sum();
            row.iter_mut().for_each(|c| *c = (*c / total).ln());
        }
        templates.logp = counts;
        let next: Vec<usize> = blocks.iter().map(|bl| templates.best(&bl.0).0).collect();
        if next == assign {
            break;
        }
        assign = next;
    }
    templates
}

/** How a level's children are ordered: rounds of corners, edges, insides (b × b under each parent), or raster over a root. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    Rounds,
    Raster,
}

/** For a position along one axis of a parent of b: whether on its border, the part's local coordinate, the part's width. */
fn axis(u: usize, b: usize) -> (bool, usize, usize) {
    if u == 0 || u == b - 1 { (true, (u == 0) as usize, 2) } else { (false, u - 1, b - 2) }
}

/** The round of position (u, v) and its index in its part's raster order. */
fn turn(u: usize, v: usize, b: usize) -> (usize, usize) {
    let ((bx, lx, lw), (by, ly, _)) = (axis(u, b), axis(v, b));
    ((!bx) as usize + (!by) as usize, ly * lw + lx)
}

/** The sides of a cell: −x, −y, +x, +y. */
const SIDES: [(i64, i64); 4] = [(-1, 0), (0, -1), (1, 0), (0, 1)];

/** Whether the neighbour on `side` of a child at (u, v) under a parent of b is decided before it. */
fn decided_before(order: Order, u: usize, v: usize, b: usize, side: usize) -> bool {
    match order {
        Order::Raster => side < 2,
        Order::Rounds => {
            let (dx, dy) = SIDES[side];
            let (nu, nv) = ((u as i64 + dx).rem_euclid(b as i64) as usize, (v as i64 + dy).rem_euclid(b as i64) as usize);
            turn(nu, nv, b) < turn(u, v, b)
        }
    }
}

/** A level's count tables, by context, backing off. */
pub struct Level {
    pub b: usize,
    pub n: usize,
    pub order: Order,
    tables: HashMap<u64, Vec<u32>>,
    prior: Vec<f64>,
}

/** A context's key: its tier (how much of the context) and its fields, 6 bits each. */
fn key(tier: u64, fields: &[u16]) -> u64 {
    fields.iter().fold(tier, |k, f| k << 6 | *f as u64)
}

/** What a child's label is drawn given: its parent, position, the parents across its edges, its neighbours decided before it. */
struct Context {
    parent: u16,
    pos: u16,
    across: [u16; 2],
    neighbours: [u16; 4],
}

impl Context {
    /** Its keys, the most specific first. */
    fn keys(&self) -> [u64; 4] {
        let [w, s, e, n] = self.neighbours;
        [
            key(1, &[self.parent, self.pos, self.across[0], self.across[1], w, s, e, n]),
            key(2, &[self.parent, self.pos, w, s, e, n]),
            key(3, &[self.parent, self.pos]),
            key(4, &[self.parent]),
        ]
    }
}

impl Level {
    fn new(b: usize, n: usize, order: Order) -> Level {
        Level { b, n, order, tables: HashMap::new(), prior: vec![1.0; n] }
    }

    /**
     * The context of child (x, y) of `label`, its parents in `parent` (a
     * cell per b × b children); `known` says if the neighbour on a side is
     * known by then (decided, or beyond the crop).
     */
    fn context(
        &self,
        x: i64,
        y: i64,
        label: &dyn Fn(i64, i64) -> u16,
        parent: &dyn Fn(i64, i64) -> u16,
        known: &dyn Fn(usize) -> bool,
    ) -> Context {
        let b = self.b as i64;
        let (u, v) = (x.rem_euclid(b) as usize, y.rem_euclid(b) as usize);
        let (px, py) = (x.div_euclid(b), y.div_euclid(b));
        let across_x = if self.order == Order::Rounds && u == 0 {
            parent(px - 1, py)
        } else if self.order == Order::Rounds && u == self.b - 1 {
            parent(px + 1, py)
        } else {
            NONE
        };
        let across_y = if self.order == Order::Rounds && v == 0 {
            parent(px, py - 1)
        } else if self.order == Order::Rounds && v == self.b - 1 {
            parent(px, py + 1)
        } else {
            NONE
        };
        let mut neighbours = [UNKNOWN; 4];
        for (side, (dx, dy)) in SIDES.into_iter().enumerate() {
            if known(side) {
                neighbours[side] = label(x + dx, y + dy);
            }
        }
        Context { parent: parent(px, py), pos: (v * self.b + u) as u16, across: [across_x, across_y], neighbours }
    }

    /** Counts `children` under `parents` (cells beyond either are OUTSIDE). */
    fn count(&mut self, children: &Image, parents: &Image) {
        let at = |im: &Image, x: i64, y: i64| {
            if x < 0 || y < 0 || x as usize >= im.w || y as usize >= im.h { OUTSIDE } else { im.at(x as usize, y as usize) }
        };
        let b = self.b as i64;
        for y in 0..children.h as i64 {
            for x in 0..children.w as i64 {
                let (u, v) = (x.rem_euclid(b) as usize, y.rem_euclid(b) as usize);
                let known = |side: usize| {
                    let (dx, dy) = SIDES[side];
                    at(children, x + dx, y + dy) == OUTSIDE || decided_before(self.order, u, v, self.b, side)
                };
                let c = self.context(x, y, &|x, y| at(children, x, y), &|x, y| at(parents, x, y), &known);
                let l = children.at(x as usize, y as usize) as usize;
                self.prior[l] += 1.0;
                for k in c.keys() {
                    self.tables.entry(k).or_insert_with(|| vec![0; self.n])[l] += 1;
                }
            }
        }
    }

    /** The chance of each label in `c`: Witten–Bell from the prior up through the keys. */
    fn distribution(&self, c: &Context) -> Vec<f64> {
        let total: f64 = self.prior.iter().sum();
        let mut p: Vec<f64> = self.prior.iter().map(|x| x / total).collect();
        for k in c.keys().iter().rev() {
            if let Some(counts) = self.tables.get(k) {
                let n: f64 = counts.iter().map(|c| *c as f64).sum();
                let t = counts.iter().filter(|c| **c > 0).count() as f64;
                p.iter_mut().zip(counts).for_each(|(p, c)| *p = (*c as f64 + t * *p) / (n + t));
            }
        }
        p
    }

    /** The log chance of `children` under `parents`, nats per child, as if each were drawn in order given the true ones before it. */
    pub fn log_likelihood(&self, children: &Image, parents: &Image) -> f64 {
        let at = |im: &Image, x: i64, y: i64| {
            if x < 0 || y < 0 || x as usize >= im.w || y as usize >= im.h { OUTSIDE } else { im.at(x as usize, y as usize) }
        };
        let b = self.b as i64;
        let mut sum = 0.0;
        for y in 0..children.h as i64 {
            for x in 0..children.w as i64 {
                let (u, v) = (x.rem_euclid(b) as usize, y.rem_euclid(b) as usize);
                let known = |side: usize| {
                    let (dx, dy) = SIDES[side];
                    at(children, x + dx, y + dy) == OUTSIDE || decided_before(self.order, u, v, self.b, side)
                };
                let c = self.context(x, y, &|x, y| at(children, x, y), &|x, y| at(parents, x, y), &known);
                sum += self.distribution(&c)[children.at(x as usize, y as usize) as usize].max(1e-12).ln();
            }
        }
        sum / (children.w * children.h) as f64
    }
}

/** A model: templates per level, and a level of tables each, finest first; the last refines the root over a crop. */
pub struct Model {
    pub templates: Vec<Templates>,
    pub levels: Vec<Level>,
    /** The crop's side in cells of the finest level. */
    pub crop: usize,
}

/** A crop's images at each level, finest first. */
pub fn pyramid(templates: &[Templates], image: &Image) -> Vec<Image> {
    let mut out = vec![image.clone()];
    for t in templates {
        let next = t.coarsen(out.last().expect("a level"));
        out.push(next);
    }
    out
}

impl Model {
    /** Templates of `ks` (each of b × b of the level below), learned from `crops` (labels below `n`), and the tables over them. */
    pub fn learn(crops: &[Image], n: usize, b: usize, ks: &[usize], seed: u64) -> Model {
        let mut templates: Vec<Templates> = Vec::new();
        let mut images: Vec<Image> = crops.to_vec();
        let mut labels = n;
        for (i, k) in ks.iter().enumerate() {
            let t = fit(&images, labels, b, *k, hash64(seed, i as i64, 0, 0));
            images = images.iter().map(|im| t.coarsen(im)).collect();
            labels = t.k;
            templates.push(t);
        }
        let crop = crops[0].w;
        let mut levels: Vec<Level> =
            (0..ks.len()).map(|i| Level::new(b, if i == 0 { n } else { templates[i - 1].k }, Order::Rounds)).collect();
        let top = crop / b.pow(ks.len() as u32);
        levels.push(Level::new(top, templates.last().map_or(n, |t| t.k), Order::Raster));
        for image in crops {
            let p = pyramid(&templates, image);
            for (i, level) in levels.iter_mut().enumerate() {
                let parents = p.get(i + 1).cloned().unwrap_or_else(|| Image::filled(1, 1, 0));
                level.count(&p[i], &parents);
            }
        }
        Model { templates, levels, crop }
    }

    /** Nats per child of each level of `image` given the true level above (held-out likelihood). */
    pub fn log_likelihood(&self, image: &Image) -> Vec<f64> {
        let p = pyramid(&self.templates, image);
        self.levels
            .iter()
            .enumerate()
            .map(|(i, l)| l.log_likelihood(&p[i], &p.get(i + 1).cloned().unwrap_or_else(|| Image::filled(1, 1, 0))))
            .collect()
    }

    /** The side of the crop in cells of level `k`. */
    fn side(&self, k: usize) -> i64 {
        (self.crop / self.levels[..k].iter().map(|l| l.b).product::<usize>()) as i64
    }

    /** Labels of level `k` (0 the finest) over the window (x0, y0) w × h of the crop, alone; OUTSIDE beyond it. */
    pub fn sample(&self, seed: u64, k: usize, x0: i64, y0: i64, w: usize, h: usize) -> Image {
        let level = &self.levels[k];
        let side = self.side(k);
        let draw = |c: &Context, x: i64, y: i64| {
            let p = level.distribution(c);
            let mut u = (hash64(seed, x, y, k as i64) >> 11) as f64 / (1u64 << 53) as f64;
            for (l, q) in p.iter().enumerate() {
                u -= q;
                if u < 0.0 {
                    return l as u16;
                }
            }
            (p.len() - 1) as u16
        };
        if level.order == Order::Raster {
            let s = side as usize;
            let mut all = Image::filled(s, s, 0);
            for y in 0..side {
                for x in 0..side {
                    let at =
                        |x: i64, y: i64| if x < 0 || y < 0 || x >= side || y >= side { OUTSIDE } else { all.at(x as usize, y as usize) };
                    let known = |side_: usize| {
                        let (dx, dy) = SIDES[side_];
                        at(x + dx, y + dy) == OUTSIDE || side_ < 2
                    };
                    let c = level.context(x, y, &at, &|_, _| 0, &known);
                    let l = draw(&c, x, y);
                    all.set(x as usize, y as usize, l);
                }
            }
            return window_of(&all, x0, y0, w, h);
        }
        let b = level.b as i64;
        let (pi0, pj0) = (x0.div_euclid(b) - 1, y0.div_euclid(b) - 1);
        let (pi1, pj1) = ((x0 + w as i64 - 1).div_euclid(b) + 1, (y0 + h as i64 - 1).div_euclid(b) + 1);
        let parents = self.sample(seed, k + 1, pi0, pj0, (pi1 - pi0 + 1) as usize, (pj1 - pj0 + 1) as usize);
        let parent = |px: i64, py: i64| {
            let (i, j) = (px - pi0, py - pj0);
            if i < 0 || j < 0 || i as usize >= parents.w || j as usize >= parents.h { OUTSIDE } else { parents.at(i as usize, j as usize) }
        };
        let (cx0, cy0) = (pi0 * b, pj0 * b);
        let (cw, ch) = (parents.w * level.b, parents.h * level.b);
        let mut canvas = Image::filled(cw, ch, UNKNOWN);
        let outside = |x: i64, y: i64| x < 0 || y < 0 || x >= side || y >= side;
        for y in 0..ch {
            for x in 0..cw {
                if outside(cx0 + x as i64, cy0 + y as i64) {
                    canvas.set(x, y, OUTSIDE);
                }
            }
        }
        let parts = (level.b - 2).max(2) * (level.b - 2).max(2);
        for round in 0..3 {
            for index in 0..parts {
                for y in 0..ch {
                    for x in 0..cw {
                        let (gx, gy) = (cx0 + x as i64, cy0 + y as i64);
                        let (u, v) = (gx.rem_euclid(b) as usize, gy.rem_euclid(b) as usize);
                        if turn(u, v, level.b) != (round, index) || outside(gx, gy) {
                            continue;
                        }
                        let at = |ax: i64, ay: i64| {
                            let (i, j) = (ax - cx0, ay - cy0);
                            if outside(ax, ay) {
                                OUTSIDE
                            } else if i < 0 || j < 0 || i as usize >= cw || j as usize >= ch {
                                UNKNOWN
                            } else {
                                canvas.at(i as usize, j as usize)
                            }
                        };
                        let known = |s: usize| {
                            let (dx, dy) = SIDES[s];
                            at(gx + dx, gy + dy) == OUTSIDE || decided_before(Order::Rounds, u, v, level.b, s)
                        };
                        let c = level.context(gx, gy, &at, &parent, &known);
                        let l = draw(&c, gx, gy);
                        canvas.set(x, y, l);
                    }
                }
            }
        }
        window_of(&canvas, x0 - cx0, y0 - cy0, w, h)
    }
}

/** The window (x0, y0) w × h of `image`, OUTSIDE beyond it. */
fn window_of(image: &Image, x0: i64, y0: i64, w: usize, h: usize) -> Image {
    let mut out = Image::filled(w, h, OUTSIDE);
    for y in 0..h {
        for x in 0..w {
            let (i, j) = (x0 + x as i64, y0 + y as i64);
            if i >= 0 && j >= 0 && (i as usize) < image.w && (j as usize) < image.h {
                out.set(x, y, image.at(i as usize, j as usize));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Crops of 64 of stripes: columns of label 1 two wide every 8, at a shift per crop, on 0. */
    fn crops() -> Vec<Image> {
        (0..6).map(|s| Image::new(64, 64, (0..64 * 64).map(|i| ((i % 64 + s) % 8 < 2) as u16).collect())).collect()
    }

    #[test]
    fn templates_tell_where_in_the_block() {
        let t = fit(&crops(), 2, 4, 4, 1);
        let coarse = t.coarsen(&crops()[0]);
        // Blocks of 4 hold the stripe at the same place in every other block along x, the same down y.
        assert_eq!(coarse.at(0, 0), coarse.at(2, 0));
        assert_eq!(coarse.at(0, 0), coarse.at(0, 5));
        assert_ne!(coarse.at(0, 0), coarse.at(1, 0));
    }

    #[test]
    fn a_box_alone_is_the_same_box_of_the_whole() {
        let model = Model::learn(&crops(), 2, 4, &[4, 3], 1);
        let whole = model.sample(5, 0, 0, 0, 64, 64);
        for (x, y, w, h) in [(0, 0, 13, 9), (37, 51, 20, 7), (63, 63, 1, 1), (5, 3, 40, 40)] {
            assert_eq!(model.sample(5, 0, x, y, w, h), whole.crop(x as usize, y as usize, w, h), "box {x} {y} {w} {h}");
        }
    }
}
