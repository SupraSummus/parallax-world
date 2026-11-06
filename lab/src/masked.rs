//! A model of any grid of labels, flat or 3D, learned from examples.
//! It knows nothing of what the labels mean or what shapes they make.
//!
//! Codes: a cell of level ℓ covers 2^ℓ label cells per axis, and its code
//! is a k-means cluster of the label shares in its sub-blocks (√, so thin
//! things count), so it says where in the cell things are.
//!
//! Drawing: level ℓ from level ℓ + 1 (the top from its position on the
//! axes in `position`). Each cell is drawn once, at a step chosen by a hash,
//! by a small network that sees its parent's codes and its neighbours as
//! they are at that step, drawn or still masked. Trained by masking
//! neighbours as at a random step, as masked image models are.
//!
//! A cell depends on a bounded box at every level, so `sample` draws a
//! window from only those boxes, the same as that window of the whole.

use worldgen::noise::{Rng, hash64, stream, unit};

use crate::net::{self, Net, Pass, masked_after, step_of};
use crate::plan::Plan;
use crate::vol::Vol;

#[derive(Clone, Debug)]
pub struct Options {
    /** Levels of codes above the labels. */
    pub levels: usize,
    /** Codes per level. */
    pub codes: usize,
    /** Sub-blocks a code's cell is described by, per axis (fewer where the cell is smaller). */
    pub subdivisions: usize,
    /** The neighbours a cell sees, per axis (0 on a flat one). */
    pub radius: [usize; 3],
    pub steps: usize,
    pub hidden: usize,
    /** Training examples per level and epoch, and epochs. */
    pub examples: usize,
    pub epochs: usize,
    /** The axes whose absolute position the top level sees. */
    pub position: [bool; 3],
    /** Whether to learn from each volume's 8 turns and mirrors about y too: for scenes with no way they face. */
    pub augment: bool,
    pub seed: u64,
}

/** A cell's state beyond its level's labels: still masked, beyond the volume. */
fn mask(n: usize) -> u16 {
    n as u16
}
fn outside(n: usize) -> u16 {
    n as u16 + 1
}
/** A parent's state beyond the volume: parents are never masked. */
fn parent_outside(n: usize) -> u16 {
    n as u16
}

/** The codes of one level: k-means centroids of cells' sub-block shares. */
pub struct Codebook {
    /** A cell's side and its sub-blocks' (per axis), and how many of these per axis, in label cells. */
    side: [usize; 3],
    sub: [usize; 3],
    grid: [usize; 3],
    n: usize,
    k: usize,
    dim: usize,
    centroids: Vec<f32>,
}

impl Codebook {
    fn new(level: usize, factor: [usize; 3], n: usize, k: usize, subdivisions: usize) -> Codebook {
        let side = factor.map(|f| f.pow(level as u32));
        let grid = side.map(|s| s.min(subdivisions));
        let sub = [0, 1, 2].map(|a| side[a] / grid[a]);
        let dim = grid.iter().product::<usize>() * n;
        Codebook { side, sub, grid, n, k, dim, centroids: Vec::new() }
    }

    fn features(&self, v: &Vol, c: [usize; 3], out: &mut [f32]) {
        out.iter_mut().for_each(|o| *o = 0.0);
        let cells = (self.sub[0] * self.sub[1] * self.sub[2]) as f32;
        for y in 0..self.side[1] {
            for z in 0..self.side[2] {
                for x in 0..self.side[0] {
                    let l = v.at([c[0] * self.side[0] + x, c[1] * self.side[1] + y, c[2] * self.side[2] + z]) as usize;
                    let g = (x / self.sub[0]) + self.grid[0] * ((z / self.sub[2]) + self.grid[2] * (y / self.sub[1]));
                    out[g * self.n + l] += 1.0;
                }
            }
        }
        out.iter_mut().for_each(|o| *o = (*o / cells).sqrt());
    }

    fn nearest(&self, f: &[f32]) -> usize {
        net::nearest(&self.centroids, self.dim, f)
    }

    /** k-means of SAMPLE cells of `vols`, drawn at random. */
    fn fit(&mut self, vols: &[Vol], seed: u64) {
        const SAMPLE: usize = 40_000;
        let mut rng = Rng::new(seed);
        let mut points = vec![0f32; SAMPLE * self.dim];
        for t in 0..SAMPLE {
            let v = &vols[rng.below(vols.len() as u32) as usize];
            let c = [0, 1, 2].map(|a| rng.below((v.n[a] / self.side[a]) as u32) as usize);
            self.features(v, c, &mut points[t * self.dim..][..self.dim]);
        }
        self.centroids = net::kmeans(&points, self.dim, self.k, rng.next_u64());
        self.k = self.centroids.len() / self.dim;
    }

    /** `v`'s whole cells, each as its code. */
    fn codes(&self, v: &Vol) -> Vol {
        let n = [0, 1, 2].map(|a| v.n[a] / self.side[a]);
        let mut out = Vol::filled(n, 0);
        let mut f = vec![0f32; self.dim];
        for y in 0..n[1] {
            for z in 0..n[2] {
                for x in 0..n[0] {
                    self.features(v, [x, y, z], &mut f);
                    out.set([x, y, z], self.nearest(&f) as u16);
                }
            }
        }
        out
    }
}

/** One level's model: what its slots are and its network. */
struct Level {
    /** Labels (or codes) of this level, and of the level above (0 at the top). */
    n: usize,
    parent_n: usize,
    /** Neighbour offsets, the parent's offsets, child positions. */
    neighbours: Vec<[i64; 3]>,
    parents: Vec<[i64; 3]>,
    children: usize,
    /** Where each kind of slot's rows start. */
    neighbour_rows: usize,
    parent_rows: usize,
    position_rows: [usize; 3],
    step_rows: usize,
    /** Where the plan's rows start: its context's (CONTEXT_LEVELS per value), then its lines' (3 per label); a cell's side in label cells, to find them. */
    context_rows: usize,
    line_rows: usize,
    side: [usize; 3],
    rows: usize,
    /** The level's size in cells (for the top level's positions); each axis's factor to the level above; the axes whose position it sees. */
    dims: [usize; 3],
    factor: [usize; 3],
    position: [bool; 3],
    net: Net,
}

pub struct Model {
    pub options: Options,
    /** Labels at level 0; each axis's factor between levels (1 if flat); level 0's size. */
    n: usize,
    factor: [usize; 3],
    size: [usize; 3],
    codebooks: Vec<Codebook>,
    levels: Vec<Level>,
    /** Held-out cross-entropy per level, bits a cell, if given volumes to hold out. */
    pub bits: Vec<f64>,
}

/** Each context value seen as one of these many levels. */
const CONTEXT_LEVELS: usize = 5;

/** Offsets within `radius` on each axis, but the centre. */
fn offsets(radius: [usize; 3]) -> Vec<[i64; 3]> {
    let r = radius.map(|v| v as i64);
    let mut out = Vec::new();
    for y in -r[1]..=r[1] {
        for z in -r[2]..=r[2] {
            for x in -r[0]..=r[0] {
                if [x, y, z] != [0, 0, 0] {
                    out.push([x, y, z]);
                }
            }
        }
    }
    out
}

impl Level {
    /**
     * The rows of cell `p`'s plan, of a flat volume (x and z its image's x
     * and y): the context at its centre; whether each line holds none of
     * the cell, some, or at least half.
     */
    fn plan_rows(&self, plan: &Plan, p: [i64; 3], out: &mut Vec<usize>) {
        let side = |a: usize| self.side[a] as i64;
        let centre = |a: usize| (p[a] * side(a) + side(a) / 2) as i32;
        for (g, v) in plan.context.at(centre(0), centre(2)).iter().enumerate() {
            let q = ((v * (CONTEXT_LEVELS - 1) as f32).round() as usize).min(CONTEXT_LEVELS - 1);
            out.push(self.context_rows + g * CONTEXT_LEVELS + q);
        }
        if let Some(lines) = &plan.lines {
            let mut shares = vec![0f32; lines.labels.len()];
            lines.shares(p[0] * side(0), p[2] * side(2), side(0), &mut shares);
            for (k, v) in shares.iter().enumerate() {
                out.push(
                    self.line_rows
                        + 3 * k
                        + if *v == 0.0 {
                            0
                        } else if *v < 0.5 {
                            1
                        } else {
                            2
                        },
                );
            }
        }
    }

    /** The rows of cell `p`'s slots: `fine` gives a neighbour's state (mask, outside included), `parent` the level above. */
    fn rows(
        &self,
        fine: &dyn Fn([i64; 3]) -> u16,
        parent: Option<&dyn Fn([i64; 3]) -> u16>,
        p: [i64; 3],
        step: usize,
        plan: Option<&Plan>,
        out: &mut Vec<usize>,
    ) {
        out.clear();
        if let Some(plan) = plan {
            self.plan_rows(plan, p, out);
        }
        let states = self.n + 2;
        for (k, d) in self.neighbours.iter().enumerate() {
            let s = fine([p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            out.push(self.neighbour_rows + k * states + s as usize);
        }
        if let Some(parent) = parent {
            let f = self.factor.map(|v| v as i64);
            let q = [0, 1, 2].map(|a| p[a].div_euclid(f[a]));
            let r = [0, 1, 2].map(|a| p[a].rem_euclid(f[a]));
            let child = (r[0] + f[0] * (r[2] + f[2] * r[1])) as usize;
            let states = self.parent_n + 1;
            for (k, d) in self.parents.iter().enumerate() {
                let s = parent([q[0] + d[0], q[1] + d[1], q[2] + d[2]]);
                out.push(self.parent_rows + (child * self.parents.len() + k) * states + s as usize);
            }
        } else {
            for ((seen, start), v) in self.position.iter().zip(self.position_rows).zip(p) {
                if *seen {
                    out.push(start + v as usize);
                }
            }
        }
        out.push(self.step_rows + step - 1);
    }
}

impl Model {
    /** Learns from `train` (all of one size), holding `held_out` out to score each level on. */
    pub fn learn(train: &[Vol], held_out: &[Vol], n: usize, options: Options) -> Model {
        Model::learn_guided(train, held_out, n, options, &[], &[])
    }

    /**
     * As `learn`, every cell also seeing the plan (plan.rs) of its place
     * in its volume, given per volume (flat ones), so that it can be drawn
     * to a plan asked for (`sample_guided`). Lines in the plans should be
     * what the volumes show there: the model learns what is around them.
     */
    pub fn learn_guided(train: &[Vol], held_out: &[Vol], n: usize, options: Options, plans: &[Plan], held_plans: &[Plan]) -> Model {
        assert!(plans.is_empty() || (plans.len() == train.len() && held_plans.len() == held_out.len() && !options.augment));
        let context_dim = plans.first().map_or(0, |p| p.context.dim);
        let lines = plans.first().and_then(|p| p.lines.as_ref()).map_or(0, |l| l.labels.len());
        let turned: Vec<Vol>;
        let train = if options.augment {
            turned = train.iter().flat_map(|v| (0..8).map(|t| v.turned(t))).collect();
            &turned[..]
        } else {
            train
        };
        let size = train[0].n;
        assert!(train.iter().chain(held_out).all(|v| v.n == size), "volumes of one size");
        let factor: [usize; 3] = size.map(|s| if s == 1 { 1 } else { 2 });
        let radius = [0, 1, 2].map(|a| if size[a] == 1 { 0 } else { options.radius[a] });
        for a in 0..3 {
            assert!(size[a].is_multiple_of(factor[a].pow(options.levels as u32)), "whole cells at the top level");
        }
        let mut codebooks = Vec::new();
        let mut pyramids: Vec<Vec<Vol>> = train.iter().map(|v| vec![v.clone()]).collect();
        let mut held: Vec<Vec<Vol>> = held_out.iter().map(|v| vec![v.clone()]).collect();
        for l in 1..=options.levels {
            let mut cb = Codebook::new(l, factor, n, options.codes, options.subdivisions);
            cb.fit(train, stream(options.seed, 100 + l as u64));
            for p in pyramids.iter_mut().chain(held.iter_mut()) {
                let codes = cb.codes(&p[0]);
                p.push(codes);
            }
            codebooks.push(cb);
        }
        let mut model = Model { options: options.clone(), n, factor, size, codebooks, levels: Vec::new(), bits: Vec::new() };
        let top = options.levels;
        for l in 0..=top {
            let level_n = if l == 0 { n } else { model.codebooks[l - 1].k };
            let parent_n = if l == top { 0 } else { model.codebooks[l].k };
            let dims = [0, 1, 2].map(|a| size[a] / factor[a].pow(l as u32));
            let neighbours = offsets(radius);
            let parents = if l == top { Vec::new() } else { offsets(factor.map(|f| f - 1)).into_iter().chain([[0, 0, 0]]).collect() };
            let children: usize = factor.iter().product();
            let neighbour_rows = 0;
            let parent_rows = neighbour_rows + neighbours.len() * (level_n + 2);
            let mut at = parent_rows + if l == top { 0 } else { children * parents.len() * (parent_n + 1) };
            let mut position_rows = [0; 3];
            for a in 0..3 {
                position_rows[a] = at;
                if l == top && options.position[a] {
                    at += dims[a];
                }
            }
            let context_rows = at;
            at += context_dim * CONTEXT_LEVELS;
            let line_rows = at;
            at += 3 * lines;
            let step_rows = at;
            let rows = at + options.steps;
            let net = Net::new(rows, options.hidden, level_n, stream(options.seed, 200 + l as u64));
            let mut level = Level {
                n: level_n,
                parent_n,
                neighbours,
                parents,
                children,
                neighbour_rows,
                parent_rows,
                position_rows,
                step_rows,
                context_rows,
                line_rows,
                side: factor.map(|f| f.pow(l as u32)),
                rows,
                dims,
                factor,
                position: if l == top { options.position } else { [false; 3] },
                net,
            };
            let fine: Vec<&Vol> = pyramids.iter().map(|p| &p[l]).collect();
            let coarse: Vec<Option<&Vol>> = pyramids.iter().map(|p| p.get(l + 1).filter(|_| l < top)).collect();
            model.train(&mut level, &fine, &coarse, plans, stream(options.seed, 300 + l as u64));
            let hf: Vec<&Vol> = held.iter().map(|p| &p[l]).collect();
            let hc: Vec<Option<&Vol>> = held.iter().map(|p| p.get(l + 1).filter(|_| l < top)).collect();
            if !hf.is_empty() {
                model.bits.push(model.score(&level, &hf, &hc, held_plans, stream(options.seed, 400 + l as u64)));
            }
            model.levels.push(level);
        }
        model
    }

    /** A training example: a random cell, its neighbours masked as at a random step; its slots' rows and its label. */
    fn example(
        &self,
        level: &Level,
        fine: &[&Vol],
        coarse: &[Option<&Vol>],
        plans: &[Plan],
        rng: &mut Rng,
        rows: &mut Vec<usize>,
    ) -> usize {
        let i = rng.below(fine.len() as u32) as usize;
        let v = fine[i];
        let p = [0, 1, 2].map(|a| rng.below(v.n[a] as u32) as i64);
        let steps = self.options.steps;
        let step = 1 + rng.below(steps as u32) as usize;
        let keep = 1.0 - masked_after(step - 1, steps);
        let n = level.n;
        // The neighbours' masks drawn from a seed of this example, so each is one draw.
        let salt = rng.next_u64();
        let fine_state = |q: [i64; 3]| match v.get(q) {
            None => outside(n),
            Some(l) => {
                if unit((hash64(salt, q[0], q[1], q[2]) >> 32) as u32) < keep {
                    l
                } else {
                    mask(n)
                }
            }
        };
        let parent_n = level.parent_n;
        let parent_state = |q: [i64; 3]| coarse[i].and_then(|c| c.get(q)).unwrap_or(parent_outside(parent_n));
        let parent: Option<&dyn Fn([i64; 3]) -> u16> = if coarse[i].is_some() { Some(&parent_state) } else { None };
        level.rows(&fine_state, parent, p, step, plans.get(i), rows);
        v.get(p).unwrap_or(0) as usize
    }

    fn train(&self, level: &mut Level, fine: &[&Vol], coarse: &[Option<&Vol>], plans: &[Plan], seed: u64) {
        let mut rng = Rng::new(seed);
        let total = self.options.examples * self.options.epochs;
        let Level { net, .. } = level;
        let mut net = std::mem::replace(net, Net::new(0, 1, 1, 0));
        net::train(&mut net, total, &mut |rows| self.example(level, fine, coarse, plans, &mut rng, rows));
        level.net = net;
    }

    /** Mean cross-entropy, bits, of cells of `fine` masked as in training. */
    fn score(&self, level: &Level, fine: &[&Vol], coarse: &[Option<&Vol>], plans: &[Plan], seed: u64) -> f64 {
        let mut rng = Rng::new(seed);
        net::bits(&level.net, &mut |rows| self.example(level, fine, coarse, plans, &mut rng, rows))
    }

    /** The size of what the model draws: its training volumes'. */
    pub fn size(&self) -> [usize; 3] {
        self.size
    }

    /**
     * The window `at`, size `n`, of a volume of the training volumes' size
     * drawn from `seed`: only the boxes each level's cells in it depend on
     * are drawn, so it is the same as that window of the whole.
     */
    pub fn sample(&self, seed: u64, at: [usize; 3], n: [usize; 3]) -> Vol {
        self.sample_guided(seed, at, n, None)
    }

    /** As `sample`, drawn to `target`, a plan over the whole volume (of a model learned with plans), its lines as they are. */
    pub fn sample_guided(&self, seed: u64, at: [usize; 3], n: [usize; 3], target: Option<&Plan>) -> Vol {
        self.draw(seed, at, n, target, None)
    }

    /**
     * As `sample_guided` over the whole volume, but levels `from` and up
     * are `real`'s codes, not drawn: how well the levels below follow
     * what is above them, given it right.
     */
    pub fn sample_given(&self, seed: u64, target: Option<&Plan>, real: &Vol, from: usize) -> Vol {
        assert_eq!(real.n, self.size, "a volume of the model's size");
        self.draw(seed, [0; 3], self.size, target, Some((real, from)))
    }

    fn draw(&self, seed: u64, at: [usize; 3], n: [usize; 3], target: Option<&Plan>, given: Option<(&Vol, usize)>) -> Vol {
        let top = self.options.levels;
        let steps = self.options.steps as i64;
        let radius: [i64; 3] = [0, 1, 2].map(|a| if self.size[a] == 1 { 0 } else { self.options.radius[a] as i64 });
        let f = self.factor.map(|v| v as i64);
        // Per level, the box wanted (lo, hi), clipped to the level's cells.
        let clip = |b: ([i64; 3], [i64; 3]), dims: [usize; 3]| -> ([i64; 3], [i64; 3]) {
            ([0, 1, 2].map(|a| b.0[a].max(0)), [0, 1, 2].map(|a| b.1[a].min(dims[a] as i64)))
        };
        let widen = |b: ([i64; 3], [i64; 3]), w: [i64; 3]| ([0, 1, 2].map(|a| b.0[a] - w[a]), [0, 1, 2].map(|a| b.1[a] + w[a]));
        let mut wanted = vec![(at.map(|v| v as i64), [0, 1, 2].map(|a| (at[a] + n[a]) as i64))];
        for l in 0..top {
            let dims = self.levels[l].dims;
            let first = clip(widen(wanted[l], radius.map(|r| r * steps)), dims);
            let parents = ([0, 1, 2].map(|a| first.0[a].div_euclid(f[a])), [0, 1, 2].map(|a| (first.1[a] - 1).div_euclid(f[a]) + 1));
            let up = clip(widen(parents, f.map(|v| v - 1)), self.levels[l + 1].dims);
            wanted.push(up);
        }
        let mut above: Option<Vol> = None;
        for l in (0..=top).rev() {
            if let Some((real, _)) = given.filter(|(_, from)| l >= (*from).max(1)) {
                above = Some(self.codebooks[l - 1].codes(real));
                continue;
            }
            let level = &self.levels[l];
            let dims = level.dims;
            let mut state = Vol::filled(dims, mask(level.n));
            let level_seed = stream(seed, l as u64);
            let parent_n = level.parent_n;
            let parent_state = |q: [i64; 3]| {
                let s = above.as_ref().and_then(|v| v.get(q));
                debug_assert!(s != Some(mask(parent_n)), "a parent read was never drawn");
                s.unwrap_or(parent_outside(parent_n))
            };
            let mut rows = Vec::new();
            let mut pass = Pass::default();
            let mut drawn: Vec<([usize; 3], u16)> = Vec::new();
            for s in 1..=self.options.steps {
                let (lo, hi) = clip(widen(wanted[l], radius.map(|r| r * (steps - s as i64))), dims);
                drawn.clear();
                for y in lo[1]..hi[1] {
                    for z in lo[2]..hi[2] {
                        for x in lo[0]..hi[0] {
                            let u = unit((hash64(level_seed, x, y, z) >> 32) as u32);
                            if step_of(u, self.options.steps) != s {
                                continue;
                            }
                            let fine_state = |q: [i64; 3]| {
                                let v = state.get(q).unwrap_or(outside(level.n));
                                // Masked only if not yet drawn: else the box drawn missed it.
                                debug_assert!(
                                    v != mask(level.n)
                                        || step_of(unit((hash64(level_seed, q[0], q[1], q[2]) >> 32) as u32), self.options.steps) >= s,
                                    "a neighbour read was never drawn"
                                );
                                v
                            };
                            let parent: Option<&dyn Fn([i64; 3]) -> u16> = if l < top { Some(&parent_state) } else { None };
                            let line = target.and_then(|t| t.lines.as_ref()).and_then(|lines| lines.at(x, z)).filter(|_| l == 0);
                            let label = line.unwrap_or_else(|| {
                                level.rows(&fine_state, parent, [x, y, z], s, target, &mut rows);
                                level.net.forward(&rows, &mut pass);
                                net::draw(&pass.p, hash64(level_seed ^ 0x5a5a, x, y, z))
                            });
                            drawn.push(([x, y, z].map(|v| v as usize), label));
                        }
                    }
                }
                for (p, label) in &drawn {
                    state.set(*p, *label);
                }
            }
            above = Some(state);
        }
        above.expect("level 0").crop(at, n)
    }

    /** Parameters, for the record. */
    pub fn parameters(&self) -> usize {
        self.levels.iter().map(|l| l.net.parameters()).sum()
    }

    /** Each level's rows and the children per parent, for the record. */
    pub fn describe(&self) -> String {
        self.levels
            .iter()
            .enumerate()
            .map(|(l, lv)| {
                format!(
                    "L{l}: {} labels, {} cells, {} slots ({} rows), {} children",
                    lv.n,
                    lv.dims.iter().product::<usize>(),
                    lv.neighbours.len() + lv.parents.len() + 1,
                    lv.rows,
                    lv.children
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn labels(&self) -> usize {
        self.n
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::plan::{Context, Lines};

    /**
     * Volumes of `n` striped along x, two labels, the phase turned over in
     * blocks of 4 by a hash: a cell follows its neighbours and its block,
     * so whatever a window drawn alone gets wrong around it shows in it.
     */
    pub fn volumes(n: [usize; 3], count: u64) -> Vec<Vol> {
        (0..count)
            .map(|s| {
                let mut v = Vol::filled(n, 0);
                for y in 0..n[1] {
                    for z in 0..n[2] {
                        for x in 0..n[0] {
                            v.set([x, y, z], ((x as u64 + hash64(s, x as i64 / 4, y as i64 / 4, z as i64 / 4)) % 2) as u16);
                        }
                    }
                }
                v
            })
            .collect()
    }

    fn options(radius: [usize; 3]) -> Options {
        Options {
            levels: 2,
            codes: 4,
            subdivisions: 4,
            radius,
            steps: 3,
            hidden: 8,
            examples: 20_000,
            epochs: 1,
            position: [true, false, true],
            augment: false,
            seed: 1,
        }
    }

    #[test]
    fn a_window_alone_is_that_window_of_the_whole() {
        // The window well inside, so that drawing too little around it would show.
        for (n, radius) in [([32, 1, 32], [1, 0, 1]), ([32, 8, 32], [1, 1, 1])] {
            let model = Model::learn(&volumes(n, 4), &[], 2, options(radius));
            let (at, w) = ([13, n[1] / 3, 11], [6, n[1].div_ceil(2), 7]);
            assert_eq!(model.sample(7, at, w), model.sample(7, [0; 3], n).crop(at, w), "{n:?}");
        }
    }

    #[test]
    fn a_window_drawn_to_a_plan_alone_is_that_window_of_the_whole() {
        let n = [32, 1, 32];
        let vols = volumes(n, 4);
        let plan = |v: &Vol| {
            let map = v.to_image();
            Plan { context: Context::of(&map, &[&[1]], 4, 1), lines: Some(Lines::of(&map, &[1])) }
        };
        let plans: Vec<Plan> = vols.iter().map(plan).collect();
        let model = Model::learn_guided(&vols, &[], 2, options([1, 0, 1]), &plans, &[]);
        let (at, w) = ([13, 0, 11], [6, 1, 7]);
        let target = Some(&plans[1]);
        assert_eq!(model.sample_guided(7, at, w, target), model.sample_guided(7, [0; 3], n, target).crop(at, w));
    }
}
