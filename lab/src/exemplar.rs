//! Synthesis by coordinates (after Lefebvre and Hoppe's parallel texture
//! synthesis): each cell of each level holds the address of a cell of an
//! example and shows that cell's label, so whole pieces of the examples
//! come through: a lane, a house with its garden.
//!
//! Each level from the one above: children take their parent's address,
//! moved by where they sit in it; a hash jitters it; then a few parallel
//! passes let each cell take the candidate whose neighbourhood in its
//! example best matches the map around it. Candidates are the neighbours'
//! addresses shifted back to the cell, so pieces grow across seams, and a
//! few from a pool of similar neighbourhoods.
//!
//! The top level starts from one example, which then gets copied, or with
//! `mixed` each top cell from an example of its own. Every step reads a
//! bounded neighbourhood, so a window drawn alone is that window of the
//! whole.

use worldgen::noise::{Rng, hash64, stream, unit};

use crate::image::Image;
use crate::net;
use crate::plan::{Lines, Plan};

#[derive(Clone, Debug)]
pub struct Options {
    /** Levels above the labels'. */
    pub levels: usize,
    /** Correction passes per level. */
    pub passes: usize,
    /** Per level, coarsest first: how far coordinates are jittered, in the level's cells. */
    pub jitter: Vec<f64>,
    /** Clusters of neighbourhoods per level, and candidates drawn from a cell's cluster (none at the two finest levels). */
    pub clusters: usize,
    pub pool: usize,
    /** Weight of a candidate's distance from the cell's place, in crops squared. */
    pub position: f64,
    /** Whether the top level starts from an example per cell rather than one for all. */
    pub mixed: bool,
    /** Weight of a candidate's context's distance from the plan's (`sample_guided`), squared. */
    pub guide: f64,
    /** Weight of the distance between the shares of lines around a candidate in its example and around its place in the plan, squared. */
    pub lines: f64,
    pub seed: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Coord {
    pub e: u16,
    pub x: i32,
    pub y: i32,
}

/** The examples, and per level the pools of coordinates by cluster of their neighbourhoods. */
pub struct Exemplars {
    pub options: Options,
    n: usize,
    w: usize,
    h: usize,
    labels: Vec<Image>,
    /** Per example, a summed-area table per label, wrapping (boxes are under 2^16 cells). */
    sat: Vec<Vec<u16>>,
    levels: Vec<(Vec<f32>, Vec<Vec<Coord>>)>,
    /** Each example's plan, and coordinates by cluster of their contexts (`with_plans`). */
    plans: Vec<Plan>,
    by_context: (Vec<f32>, Vec<Vec<Coord>>),
}

fn distance2(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| ((x - y) * (x - y)) as f64).sum()
}

const DIM_CELLS: usize = 9;

impl Exemplars {
    pub fn new(examples: &[Image], n: usize, options: Options) -> Exemplars {
        let (w, h) = (examples[0].w, examples[0].h);
        assert!(examples.iter().all(|e| e.w == w && e.h == h), "examples of one size");
        assert!(w.is_multiple_of(1 << options.levels) && h.is_multiple_of(1 << options.levels), "whole cells at the top");
        assert!(examples.len() < u16::MAX as usize && (1 << options.levels) * (1 << options.levels) < 1 << 16);
        let sat = examples
            .iter()
            .map(|e| {
                let mut t = vec![0u16; (w + 1) * (h + 1) * n];
                for y in 0..h {
                    for x in 0..w {
                        for l in 0..n {
                            let at = |x: usize, y: usize| ((y * (w + 1)) + x) * n + l;
                            let own = (e.at(x, y) as usize == l) as u16;
                            t[at(x + 1, y + 1)] = own.wrapping_add(t[at(x, y + 1)]).wrapping_add(t[at(x + 1, y)]).wrapping_sub(t[at(x, y)]);
                        }
                    }
                }
                t
            })
            .collect();
        let mut ex = Exemplars {
            options: options.clone(),
            n,
            w,
            h,
            labels: examples.to_vec(),
            sat,
            levels: Vec::new(),
            plans: Vec::new(),
            by_context: (Vec::new(), Vec::new()),
        };
        let dim = DIM_CELLS * n;
        for l in 0..=options.levels {
            if !ex.pools_at(l) {
                ex.levels.push((Vec::new(), Vec::new()));
                continue;
            }
            let mut rng = Rng::new(stream(options.seed, 50 + l as u64));
            const SAMPLE: usize = 30_000;
            let coords: Vec<Coord> = (0..SAMPLE)
                .map(|_| Coord { e: rng.below(examples.len() as u32) as u16, x: rng.below(w as u32) as i32, y: rng.below(h as u32) as i32 })
                .collect();
            let mut points = vec![0f32; SAMPLE * dim];
            for (i, c) in coords.iter().enumerate() {
                ex.neighbourhood(l, *c, &mut points[i * dim..][..dim]);
            }
            let centroids = net::kmeans(&points, dim, options.clusters, stream(options.seed, 60 + l as u64));
            let mut pools = vec![Vec::new(); centroids.len() / dim];
            for (i, c) in coords.iter().enumerate() {
                pools[net::nearest(&centroids, dim, &points[i * dim..][..dim])].push(*c);
            }
            ex.levels.push((centroids, pools));
        }
        ex
    }

    /** Gives each example its plan, so `sample_guided` can follow one; coordinates are clustered by their contexts. */
    pub fn with_plans(mut self, plans: Vec<Plan>) -> Exemplars {
        assert_eq!(plans.len(), self.labels.len(), "a plan per example");
        let dim = plans[0].context.dim;
        let mut rng = Rng::new(stream(self.options.seed, 70));
        const SAMPLE: usize = 30_000;
        let coords: Vec<Coord> = (0..SAMPLE)
            .map(|_| Coord {
                e: rng.below(self.labels.len() as u32) as u16,
                x: rng.below(self.w as u32) as i32,
                y: rng.below(self.h as u32) as i32,
            })
            .collect();
        let points: Vec<f32> = coords.iter().flat_map(|c| plans[c.e as usize].context.at(c.x, c.y).to_vec()).collect();
        let centroids = net::kmeans(&points, dim, 64, stream(self.options.seed, 71));
        let mut pools = vec![Vec::new(); centroids.len() / dim];
        for (i, c) in coords.iter().enumerate() {
            pools[net::nearest(&centroids, dim, &points[i * dim..][..dim])].push(*c);
        }
        // Clusters left empty (contexts repeat: all forest, all fields) are dropped, so a target never lands in one.
        let kept: Vec<usize> = (0..pools.len()).filter(|k| !pools[*k].is_empty()).collect();
        let centroids = kept.iter().flat_map(|k| centroids[k * dim..][..dim].to_vec()).collect();
        let pools = kept.iter().map(|k| std::mem::take(&mut pools[*k])).collect();
        self.plans = plans;
        self.by_context = (centroids, pools);
        self
    }

    fn context(&self, c: Coord) -> &[f32] {
        self.plans[c.e as usize].context.at(c.x, c.y)
    }

    /**
     * How far the lines around `c` in its example are from those around
     * `place` in the plan's: the squared distances of their √ shares in the
     * 3 × 3 blocks a level's spacing apart.
     */
    fn line_cost(&self, level: usize, c: Coord, lines: &Lines, place: (i64, i64)) -> f64 {
        let (s, k) = (self.spacing(level) as i64, lines.labels.len());
        let (mut a, mut b) = (vec![0f32; k], vec![0f32; k]);
        let mut cost = 0.0;
        for (dx, dy) in OFFSETS.map(|(x, y)| (x as i64 * s - s / 2, y as i64 * s - s / 2)) {
            match &self.plans[c.e as usize].lines {
                Some(own) => own.shares(c.x as i64 + dx, c.y as i64 + dy, s, &mut a),
                None => a.iter_mut().for_each(|v| *v = 0.0),
            }
            lines.shares(place.0 + dx, place.1 + dy, s, &mut b);
            cost += a.iter().zip(&b).map(|(p, q)| ((p.sqrt() - q.sqrt()) as f64).powi(2)).sum::<f64>();
        }
        cost
    }

    /** Whether level `l` draws candidates from pools: not the two finest, where neighbours' coordinates suffice. */
    fn pools_at(&self, l: usize) -> bool {
        l + 2 <= self.options.levels && self.options.pool > 0
    }

    /** A level's spacing in example cells. */
    fn spacing(&self, level: usize) -> i32 {
        1 << (self.options.levels - level)
    }

    /** The √ shares of the labels in the h × h block about (x, y) of example e (clipped to it). */
    fn shares(&self, level: usize, c: Coord, out: &mut [f32]) {
        let s = self.spacing(level);
        let (x0, y0) = ((c.x - s / 2).clamp(0, self.w as i32 - 1), (c.y - s / 2).clamp(0, self.h as i32 - 1));
        let (x1, y1) = ((x0 + s).min(self.w as i32), (y0 + s).min(self.h as i32));
        let t = &self.sat[c.e as usize];
        let n = self.n;
        let at = |x: i32, y: i32, l: usize| t[((y as usize) * (self.w + 1) + x as usize) * n + l];
        let area = ((x1 - x0) * (y1 - y0)) as f32;
        for (l, o) in out.iter_mut().enumerate().take(n) {
            let v = at(x1, y1, l).wrapping_sub(at(x0, y1, l)).wrapping_sub(at(x1, y0, l)).wrapping_add(at(x0, y0, l));
            *o = (v as f32 / area).sqrt();
        }
    }

    /** The example's neighbourhood of a coordinate at a level: 3 × 3 blocks h apart. */
    fn neighbourhood(&self, level: usize, c: Coord, out: &mut [f32]) {
        let s = self.spacing(level);
        let n = self.n;
        for (k, (dx, dy)) in OFFSETS.iter().enumerate() {
            self.shares(level, Coord { x: c.x + dx * s, y: c.y + dy * s, ..c }, &mut out[k * n..][..n]);
        }
    }

    /** A coordinate brought into its example by reflecting at its sides (see `reflect`). */
    fn clamp(&self, c: Coord) -> Coord {
        Coord { e: c.e, x: reflect(c.x, self.w), y: reflect(c.y, self.h) }
    }

    pub fn label(&self, c: Coord) -> u16 {
        self.labels[c.e as usize].at(c.x as usize, c.y as usize)
    }

    /**
     * The window (x0, y0) w × h of a crop the examples' size drawn from
     * `seed`: its labels, and the coordinates they come from.
     */
    pub fn sample(&self, seed: u64, window: (usize, usize, usize, usize)) -> (Image, Vec<Coord>) {
        self.sample_guided(seed, window, None)
    }

    /**
     * As `sample`, drawn to `target`, a plan over the whole crop: the top
     * level starts each cell from a place in the examples of about its
     * context, every candidate pays `guide` for its distance from it and
     * `lines` for how differently lines run around it, and the plan's lines
     * are drawn over what comes out.
     */
    pub fn sample_guided(&self, seed: u64, (x0, y0, ww, wh): (usize, usize, usize, usize), target: Option<&Plan>) -> (Image, Vec<Coord>) {
        assert!(target.is_none() || !self.plans.is_empty(), "examples' plans to follow a target with");
        let top = self.options.levels;
        let subpasses = 4 * self.options.passes as i64;
        let dims = |l: usize| ((self.w >> (top - l)) as i64, (self.h >> (top - l)) as i64);
        type Rect = (i64, i64, i64, i64);
        let clip = |r: Rect, (dx, dy): (i64, i64)| (r.0.max(0), r.1.max(0), r.2.min(dx), r.3.min(dy));
        let widen = |r: Rect, m: i64| (r.0 - m, r.1 - m, r.2 + m, r.3 + m);
        // The cells each level must hold: the window's, widened by what corrections read, and their parents.
        let mut wanted: Vec<Rect> = vec![(0, 0, 0, 0); top + 1];
        wanted[top] = (x0 as i64, y0 as i64, (x0 + ww) as i64, (y0 + wh) as i64);
        for l in (1..=top).rev() {
            let r = clip(widen(wanted[l], subpasses), dims(l));
            wanted[l - 1] = (r.0.div_euclid(2), r.1.div_euclid(2), (r.2 - 1).div_euclid(2) + 1, (r.3 - 1).div_euclid(2) + 1);
        }
        let n = self.n;
        let dim = DIM_CELLS * n;
        let mut above: Vec<Coord> = Vec::new();
        let mut above_w = 0i64;
        let mut f = vec![0f32; dim];
        let mut g = vec![0f32; dim];
        let first = (hash64(seed, 0, 0, 7) % self.labels.len() as u64) as u16;
        for (l, &want) in wanted.iter().enumerate() {
            let (dw, dh) = dims(l);
            let s = self.spacing(l) as i64;
            let level_seed = stream(seed, l as u64);
            let mut state = vec![UNDRAWN; (dw * dh) as usize];
            let idx = |x: i64, y: i64| (y * dw + x) as usize;
            let r0 = clip(widen(want, subpasses), (dw, dh));
            let jitter = self.options.jitter.get(l).copied().unwrap_or(0.0);
            for y in r0.1..r0.3 {
                for x in r0.0..r0.2 {
                    let place = ((x * s + s / 2) as i32, (y * s + s / 2) as i32);
                    let base = if let (0, Some(t)) = (l, target) {
                        // The best for the place's context of a few from the examples of about it.
                        let want = t.context.at(place.0, place.1);
                        let (centroids, pools) = &self.by_context;
                        let pool = &pools[net::nearest(centroids, want.len(), want)];
                        (0..8)
                            .map(|i| pool[(hash64(seed, x, y, 200 + i) % pool.len() as u64) as usize])
                            .min_by(|a, b| distance2(self.context(*a), want).total_cmp(&distance2(self.context(*b), want)))
                            .expect("a pool holds coordinates")
                    } else if l == 0 {
                        // One example's coarse layout, or each top cell its own example's.
                        let e = if self.options.mixed { (hash64(seed, x, y, 8) % self.labels.len() as u64) as u16 } else { first };
                        Coord { e, x: place.0, y: place.1 }
                    } else {
                        let p = above[((y / 2) * above_w + x / 2) as usize];
                        debug_assert!(p != UNDRAWN, "a parent read was never drawn");
                        Coord { e: p.e, x: p.x + ((x % 2) * s - s / 2) as i32, y: p.y + ((y % 2) * s - s / 2) as i32 }
                    };
                    let hj = hash64(level_seed, x, y, 1);
                    let j = |b: u64| ((unit((b & 0xffff_ffff) as u32) * 2.0 - 1.0) * jitter * s as f64).round() as i32;
                    state[idx(x, y)] = self.clamp(Coord { x: base.x + j(hj), y: base.y + j(hj >> 32), ..base });
                }
            }
            let mut q = 0;
            for pass in 0..self.options.passes {
                for (sx, sy) in [(0, 0), (1, 1), (1, 0), (0, 1)] {
                    q += 1;
                    let r = clip(widen(want, subpasses - q), (dw, dh));
                    for y in r.1..r.3 {
                        for x in r.0..r.2 {
                            if x % 2 != sx || y % 2 != sy {
                                continue;
                            }
                            // The map's neighbourhood: each neighbour's own block; missing beyond the crop.
                            let mut have = [false; DIM_CELLS];
                            for (k, (dx, dy)) in OFFSETS.iter().enumerate() {
                                let (u, v) = (x + *dx as i64, y + *dy as i64);
                                if u >= 0 && v >= 0 && u < dw && v < dh {
                                    have[k] = true;
                                    debug_assert!(state[idx(u, v)] != UNDRAWN, "a neighbour read was never drawn");
                                    self.shares(l, state[idx(u, v)], &mut f[k * n..][..n]);
                                }
                            }
                            let mut candidates: Vec<Coord> = vec![state[idx(x, y)]];
                            for (k, (dx, dy)) in OFFSETS.iter().enumerate() {
                                if have[k] && (*dx, *dy) != (0, 0) {
                                    let c = state[idx(x + *dx as i64, y + *dy as i64)];
                                    candidates.push(self.clamp(Coord { x: c.x - dx * s as i32, y: c.y - dy * s as i32, ..c }));
                                }
                            }
                            if self.pools_at(l) {
                                let (centroids, pools) = &self.levels[l];
                                // Neighbours beyond the crop are missing: the centre's block stands in for them here.
                                let centre = f[4 * n..][..n].to_vec();
                                for k in (0..DIM_CELLS).filter(|k| !have[*k]) {
                                    f[k * n..][..n].copy_from_slice(&centre);
                                }
                                let pool = &pools[net::nearest(centroids, dim, &f)];
                                for i in 0..self.options.pool.min(pool.len()) {
                                    let h = hash64(level_seed ^ pass as u64, x, y, 100 + i as i64);
                                    candidates.push(pool[(h % pool.len() as u64) as usize]);
                                }
                            }
                            let place = ((x * s + s / 2) as f64, (y * s + s / 2) as f64);
                            let mut best = (f64::INFINITY, candidates[0]);
                            for c in candidates {
                                self.neighbourhood(l, c, &mut g);
                                let mut cost = 0.0f64;
                                for k in 0..DIM_CELLS {
                                    if have[k] {
                                        cost += f[k * n..][..n]
                                            .iter()
                                            .zip(&g[k * n..][..n])
                                            .map(|(a, b)| ((a - b) * (a - b)) as f64)
                                            .sum::<f64>();
                                    }
                                }
                                let d2 = ((c.x as f64 - place.0).powi(2) + (c.y as f64 - place.1).powi(2)) / (self.w * self.h) as f64;
                                cost += self.options.position * d2;
                                if let Some(t) = target {
                                    cost += self.options.guide * distance2(self.context(c), t.context.at(place.0 as i32, place.1 as i32));
                                    if let Some(lines) = &t.lines {
                                        cost += self.options.lines * self.line_cost(l, c, lines, (place.0 as i64, place.1 as i64));
                                    }
                                }
                                if cost < best.0 {
                                    best = (cost, c);
                                }
                            }
                            state[idx(x, y)] = best.1;
                        }
                    }
                }
            }
            above = state;
            above_w = dw;
        }
        let mut out = Image::filled(ww, wh, 0);
        let mut coords = Vec::with_capacity(ww * wh);
        for y in 0..wh {
            for x in 0..ww {
                let c = above[(y + y0) * self.w + x + x0];
                let line = target.and_then(|t| t.lines.as_ref()).and_then(|l| l.at((x + x0) as i64, (y + y0) as i64));
                out.set(x, y, line.unwrap_or_else(|| self.label(c)));
                coords.push(c);
            }
        }
        (out, coords)
    }
}

/** What a cell holds until drawn, so that reading one too early shows (in debug builds). */
const UNDRAWN: Coord = Coord { e: u16::MAX, x: 0, y: 0 };

/** The 3 × 3 neighbourhood, centre at index 4. */
const OFFSETS: [(i32, i32); DIM_CELLS] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/**
 * How much of a draw is copied: the mean size (area-weighted, cells) of
 * its coherent pieces (neighbours whose coordinates are neighbours in one
 * example), and the share drawn from its commonest example.
 */
pub fn copying(coords: &[Coord], w: usize) -> (f64, f64) {
    let h = coords.len() / w;
    let mut links = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let c = coords[y * w + x];
            for (dx, dy) in [(1, 0), (0, 1)] {
                if x + dx < w && y + dy < h {
                    let d = coords[(y + dy) * w + x + dx];
                    if d.e == c.e && d.x == c.x + dx as i32 && d.y == c.y + dy as i32 {
                        links.push((y * w + x, (y + dy) * w + x + dx));
                    }
                }
            }
        }
    }
    (mean_piece(coords.len(), &links), dominant(coords.iter().map(|c| c.e)))
}

/** The mean size, weighted by size, of the pieces `links` join `n` cells into. */
pub fn mean_piece(n: usize, links: &[(usize, usize)]) -> f64 {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (a, b) in links {
        let (a, b) = (find(&mut parent, *a), find(&mut parent, *b));
        parent[a] = b;
    }
    let mut size = vec![0usize; n];
    for i in 0..n {
        let r = find(&mut parent, i);
        size[r] += 1;
    }
    size.iter().map(|s| (*s * *s) as f64).sum::<f64>() / n as f64
}

/** `v` brought into 0 ..= n − 1 by reflecting at the ends: clamping would give coordinates beyond one the same, and stripes. */
pub fn reflect(v: i32, n: usize) -> i32 {
    let n = n as i32;
    let m = v.rem_euclid(2 * n);
    if m < n { m } else { 2 * n - 1 - m }
}

/** The share of cells drawn from the example most of them are drawn from: near 1 is one example moved about. */
pub fn dominant(examples: impl Iterator<Item = u16>) -> f64 {
    let mut count = std::collections::HashMap::new();
    let mut total = 0usize;
    for e in examples {
        *count.entry(e).or_insert(0usize) += 1;
        total += 1;
    }
    count.values().copied().max().unwrap_or(0) as f64 / total.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Context;

    #[test]
    fn a_window_alone_is_that_window_of_the_whole() {
        let examples: Vec<Image> = crate::masked::tests::volumes([32, 1, 32], 4).iter().map(|v| v.to_image()).collect();
        let options = Options {
            levels: 2,
            passes: 1,
            jitter: vec![0.5, 0.3, 0.0],
            clusters: 4,
            pool: 2,
            position: 1.0,
            mixed: true,
            guide: 4.0,
            lines: 1.0,
            seed: 1,
        };
        let plan = |e: &Image| Plan { context: Context::of(e, &[&[1]], 4, 1), lines: Some(Lines::of(e, &[1])) };
        let ex = Exemplars::new(&examples, 2, options).with_plans(examples.iter().map(plan).collect());
        let target = plan(&examples[2]);
        for target in [None, Some(&target)] {
            let (alone, whole) = (ex.sample_guided(7, (13, 11, 6, 7), target).1, ex.sample_guided(7, (0, 0, 32, 32), target).1);
            let window: Vec<Coord> = (11..18).flat_map(|y| whole[y * 32 + 13..][..6].to_vec()).collect();
            assert_eq!(alone, window);
        }
    }
}
