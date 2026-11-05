//! The masked model (masked.rs) on the worlds' FCC lattice:
//! sites where x + y + z is even, coarse site S at 2S of the level below.
//!
//! There is no octree: the engine's coarse cells (voxel-grid.ts's
//! COARSE_CELL) overlap, so a fine site is in one cell, two or six.
//! Codes describe a cell by its 19 parts, weighted as the engine coarsens.
//! A site sees its neighbours and the coarse sites near it, each in a slot
//! of its offset, which also tells which kind of site it is.
//! `Cells::Owned` gives each fine site one parent instead, like an octree,
//! to compare.
//!
//! A volume is the box of points [0, e] per axis (e a multiple of
//! 2^(levels + 1), so turns about y keep coarse sites), labels at sites
//! only.

use worldgen::noise::{Rng, hash64, stream, unit};

use crate::net::{self, Net, Pass, masked_after, step_of};
use crate::vol::Vol;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cells {
    /** The engine's: cells overlap; a fine site is in 1, 2 or 6 of them. */
    Shared,
    /** A partition: each fine site in the cell of one parent, the least of them (x, then y, then z). */
    Owned,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub levels: usize,
    pub codes: usize,
    /** Squared distance (in the level's units) within which a site sees same-level sites, and coarse ones. */
    pub neighbours: i64,
    pub parents: i64,
    pub steps: usize,
    pub hidden: usize,
    pub examples: usize,
    pub epochs: usize,
    /** Whether the top level sees its height. */
    pub height: bool,
    /** Whether to learn from each volume's 8 turns and mirrors about y too. */
    pub augment: bool,
    pub cells: Cells,
    pub seed: u64,
}

pub fn is_site(p: [i64; 3]) -> bool {
    worldgen::grid::is_site(p[0], p[1], p[2])
}

/** A site of a box `dims` points a side, uniformly: points drawn until one is a site (half are). */
fn random_site(rng: &mut Rng, dims: [usize; 3]) -> [i64; 3] {
    loop {
        let p = [0, 1, 2].map(|a| rng.below(dims[a] as u32) as i64);
        if is_site(p) {
            return p;
        }
    }
}

/** The cell of a coarse site: its parts' offsets from 2S and weights (of 48), as voxel-grid.ts's COARSE_CELL. */
pub fn coarse_cell() -> Vec<([i64; 3], f32)> {
    let mut parts = vec![([0, 0, 0], 6.0)];
    for s in [-1, 1] {
        for t in [-1, 1] {
            parts.extend([([s, t, 0], 3.0), ([s, 0, t], 3.0), ([0, s, t], 3.0)]);
        }
    }
    for s in [-2, 2] {
        parts.extend([([s, 0, 0], 1.0), ([0, s, 0], 1.0), ([0, 0, s], 1.0)]);
    }
    parts
}

/** Whether a cell owns its part at `o` under Cells::Owned: of the fine site's parents, it is the least. */
fn owns(o: [i64; 3]) -> bool {
    // The fine site 2S + o has parents at 2S + o − o' for each o' of its kind; 2S is the least where o is the greatest.
    let w = o.iter().map(|v| v.abs()).sum::<i64>();
    match w {
        0 => true,
        // Between two: the other parent is 2S + 2o, greater where o is (first non-zero coordinate positive).
        2 if o.iter().any(|v| v.abs() == 1) => o.iter().find(|v| **v != 0).is_some_and(|v| *v > 0),
        // Among six: only the one with o = (2, 0, 0).
        _ => o == [2, 0, 0],
    }
}

/** Offsets d ≠ 0 with x + y + z even and |d|² ≤ r2. */
fn lattice_offsets(r2: i64, centre: bool) -> Vec<[i64; 3]> {
    let r = (r2 as f64).sqrt() as i64;
    let mut out = Vec::new();
    for y in -r..=r {
        for z in -r..=r {
            for x in -r..=r {
                let d = [x, y, z];
                if is_site(d) && x * x + y * y + z * z <= r2 && (centre || d != [0, 0, 0]) {
                    out.push(d);
                }
            }
        }
    }
    out
}

/** Offsets d from a fine site to coarse sites, |d|² ≤ r2 (any parity: which apply depends on the site). */
fn parent_offsets(r2: i64) -> Vec<[i64; 3]> {
    let r = (r2 as f64).sqrt() as i64;
    let mut out = Vec::new();
    for y in -r..=r {
        for z in -r..=r {
            for x in -r..=r {
                if is_site([x, y, z]) && x * x + y * y + z * z <= r2 {
                    out.push([x, y, z]);
                }
            }
        }
    }
    out
}

fn mask(n: usize) -> u16 {
    n as u16
}
fn outside(n: usize) -> u16 {
    n as u16 + 1
}

struct Level {
    n: usize,
    parent_n: usize,
    neighbours: Vec<[i64; 3]>,
    /** Offsets to coarse sites, and for each a slot. */
    parents: Vec<[i64; 3]>,
    neighbour_rows: usize,
    parent_rows: usize,
    height_rows: Option<usize>,
    step_rows: usize,
    rows: usize,
    dims: [usize; 3],
    net: Net,
}

impl Level {
    fn rows(
        &self,
        fine: &dyn Fn([i64; 3]) -> u16,
        parent: Option<&dyn Fn([i64; 3]) -> u16>,
        p: [i64; 3],
        step: usize,
        out: &mut Vec<usize>,
    ) {
        out.clear();
        let states = self.n + 2;
        for (k, d) in self.neighbours.iter().enumerate() {
            out.push(self.neighbour_rows + k * states + fine([p[0] + d[0], p[1] + d[1], p[2] + d[2]]) as usize);
        }
        if let Some(parent) = parent {
            let states = self.parent_n + 1;
            for (k, d) in self.parents.iter().enumerate() {
                let q = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
                // A coarse site at q = 2S: q even on every axis, S a site.
                if q.iter().all(|v| v.rem_euclid(2) == 0) && is_site(q.map(|v| v.div_euclid(2))) {
                    out.push(self.parent_rows + k * states + parent(q.map(|v| v.div_euclid(2))) as usize);
                }
            }
        }
        if let Some(rows) = self.height_rows {
            out.push(rows + p[1] as usize);
        }
        out.push(self.step_rows + step - 1);
    }
}

pub struct Model {
    pub options: Options,
    n: usize,
    size: [usize; 3],
    /** Per level ≥ 1, the codes' centroids (19 parts × the labels' count each). */
    centroids: Vec<Vec<f32>>,
    levels: Vec<Level>,
    pub bits: Vec<f64>,
}

/** A level's size in points for a volume of `size` points at level 0. */
fn dims_at(size: [usize; 3], level: usize) -> [usize; 3] {
    size.map(|s| (s - 1) / (1 << level) + 1)
}

/**
 * Each site's shares of the labels in its cell, per level 0 to `levels`
 * (n floats a point; points not sites 0), nesting as `cells` says.
 */
fn shares(v: &Vol, n: usize, levels: usize, cells: Cells) -> Vec<Vec<f32>> {
    let parts = coarse_cell();
    let mut out = vec![vec![0f32; v.data.len() * n]];
    for (i, l) in v.data.iter().enumerate() {
        out[0][i * n + *l as usize] = 1.0;
    }
    for level in 1..=levels {
        let (fine_dims, dims) = (dims_at(v.n, level - 1), dims_at(v.n, level));
        let index = |p: [usize; 3], d: [usize; 3]| p[0] + d[0] * (p[2] + d[2] * p[1]);
        let mut h = vec![0f32; dims.iter().product::<usize>() * n];
        for y in 0..dims[1] {
            for z in 0..dims[2] {
                for x in 0..dims[0] {
                    let s = [x as i64, y as i64, z as i64];
                    if !is_site(s) {
                        continue;
                    }
                    let at = index([x, y, z], dims) * n;
                    let mut total = 0.0;
                    for (o, w) in &parts {
                        let w = if cells == Cells::Owned { if owns(*o) { 1.0 } else { 0.0 } } else { *w };
                        let q = [2 * s[0] + o[0], 2 * s[1] + o[1], 2 * s[2] + o[2]];
                        if w == 0.0 || (0..3).any(|a| q[a] < 0 || q[a] >= fine_dims[a] as i64) {
                            continue;
                        }
                        let j = index(q.map(|v| v as usize), fine_dims) * n;
                        for l in 0..n {
                            h[at + l] += w * out[level - 1][j + l];
                        }
                        total += w;
                    }
                    h[at..at + n].iter_mut().for_each(|v| *v /= total);
                }
            }
        }
        out.push(h);
    }
    out
}

/** A coarse site's features at `level`: its 19 parts' shares one level down, √ and weighted. */
fn features(shares: &[Vec<f32>], size: [usize; 3], n: usize, level: usize, s: [i64; 3], cells: Cells, out: &mut [f32]) {
    out.iter_mut().for_each(|v| *v = 0.0);
    let fine_dims = dims_at(size, level - 1);
    for (k, (o, w)) in coarse_cell().iter().enumerate() {
        let w = if cells == Cells::Owned { if owns(*o) { 6.0 } else { 0.0 } } else { *w };
        let q = [2 * s[0] + o[0], 2 * s[1] + o[1], 2 * s[2] + o[2]];
        if w == 0.0 || (0..3).any(|a| q[a] < 0 || q[a] >= fine_dims[a] as i64) {
            continue;
        }
        let j = (q[0] as usize + fine_dims[0] * (q[2] as usize + fine_dims[2] * q[1] as usize)) * n;
        for l in 0..n {
            out[k * n + l] = (w / 6.0 * shares[level - 1][j + l]).sqrt();
        }
    }
}

impl Model {
    pub fn learn(train: &[Vol], held_out: &[Vol], n: usize, options: Options) -> Model {
        let turned: Vec<Vol>;
        let train = if options.augment {
            turned = train.iter().flat_map(|v| (0..8).map(|t| v.turned(t))).collect();
            &turned[..]
        } else {
            train
        };
        let size = train[0].n;
        let top = options.levels;
        for (a, s) in size.iter().enumerate() {
            assert!((s - 1).is_multiple_of(1 << top), "a box of whole coarse cells");
            // A turn maps x to e − x, which keeps coarse sites coarse sites only for e a multiple of 2^(levels + 1).
            assert!(!options.augment || a == 1 || (s - 1).is_multiple_of(1 << (top + 1)), "a box the turns keep");
        }
        let dim = 19 * n;
        // Codes: sample features of every level, fit, then each volume's codes.
        // Of at most FROM volumes, evenly: their shares are costly, and k-means needs only SAMPLE points.
        let mut rng = Rng::new(stream(options.seed, 1));
        const SAMPLE: usize = 40_000;
        const FROM: usize = 32;
        let from: Vec<&Vol> = train.iter().step_by(train.len().div_ceil(FROM)).collect();
        let per = SAMPLE / from.len() + 1;
        let mut points: Vec<Vec<f32>> = vec![Vec::new(); top + 1];
        let mut f = vec![0f32; dim];
        for v in from {
            let sh = shares(v, n, top, options.cells);
            for (level, pts) in points.iter_mut().enumerate().skip(1) {
                let dims = dims_at(size, level);
                for _ in 0..per {
                    let s = random_site(&mut rng, dims);
                    features(&sh, size, n, level, s, options.cells, &mut f);
                    pts.extend_from_slice(&f);
                }
            }
        }
        let centroids: Vec<Vec<f32>> =
            (1..=top).map(|l| net::kmeans(&points[l], dim, options.codes, stream(options.seed, 10 + l as u64))).collect();
        let pyramid = |v: &Vol| -> Vec<Vol> {
            let sh = shares(v, n, top, options.cells);
            let mut out = vec![v.clone()];
            for level in 1..=top {
                let dims = dims_at(size, level);
                let mut codes = Vol::filled(dims, 0);
                let mut f = vec![0f32; dim];
                for y in 0..dims[1] {
                    for z in 0..dims[2] {
                        for x in 0..dims[0] {
                            let s = [x as i64, y as i64, z as i64];
                            if is_site(s) {
                                features(&sh, size, n, level, s, options.cells, &mut f);
                                codes.set([x, y, z], net::nearest(&centroids[level - 1], dim, &f) as u16);
                            }
                        }
                    }
                }
                out.push(codes);
            }
            out
        };
        let pyramids: Vec<Vec<Vol>> = train.iter().map(pyramid).collect();
        let held: Vec<Vec<Vol>> = held_out.iter().map(pyramid).collect();
        let mut model = Model { options: options.clone(), n, size, centroids, levels: Vec::new(), bits: Vec::new() };
        for l in 0..=top {
            let level_n = if l == 0 { n } else { model.centroids[l - 1].len() / dim };
            let parent_n = if l == top { 0 } else { model.centroids[l].len() / dim };
            let neighbours = lattice_offsets(options.neighbours, false);
            let parents = if l == top { Vec::new() } else { parent_offsets(options.parents) };
            let dims = dims_at(size, l);
            let neighbour_rows = 0;
            let parent_rows = neighbours.len() * (level_n + 2);
            let mut at = parent_rows + parents.len() * (parent_n + 1);
            let height_rows = (l == top && options.height).then(|| {
                at += dims[1];
                at - dims[1]
            });
            let step_rows = at;
            let rows = at + options.steps;
            let net = Net::new(rows, options.hidden, level_n, stream(options.seed, 200 + l as u64));
            let mut level =
                Level { n: level_n, parent_n, neighbours, parents, neighbour_rows, parent_rows, height_rows, step_rows, rows, dims, net };
            let fine: Vec<&Vol> = pyramids.iter().map(|p| &p[l]).collect();
            let coarse: Vec<Option<&Vol>> = pyramids.iter().map(|p| p.get(l + 1)).collect();
            let mut rng = Rng::new(stream(options.seed, 300 + l as u64));
            let mut net = std::mem::replace(&mut level.net, Net::new(0, 1, 1, 0));
            net::train(&mut net, options.examples * options.epochs, &mut |rows| model.example(&level, &fine, &coarse, &mut rng, rows));
            level.net = net;
            if !held.is_empty() {
                let hf: Vec<&Vol> = held.iter().map(|p| &p[l]).collect();
                let hc: Vec<Option<&Vol>> = held.iter().map(|p| p.get(l + 1)).collect();
                let mut rng = Rng::new(stream(options.seed, 400 + l as u64));
                model.bits.push(net::bits(&level.net, &mut |rows| model.example(&level, &hf, &hc, &mut rng, rows)));
            }
            model.levels.push(level);
        }
        model
    }

    fn example(&self, level: &Level, fine: &[&Vol], coarse: &[Option<&Vol>], rng: &mut Rng, rows: &mut Vec<usize>) -> usize {
        let i = rng.below(fine.len() as u32) as usize;
        let v = fine[i];
        let p = random_site(rng, v.n);
        let steps = self.options.steps;
        let step = 1 + rng.below(steps as u32) as usize;
        let keep = 1.0 - masked_after(step - 1, steps);
        let n = level.n;
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
        let parent_state = |q: [i64; 3]| coarse[i].and_then(|c| c.get(q)).unwrap_or(parent_n as u16);
        let parent: Option<&dyn Fn([i64; 3]) -> u16> = if coarse[i].is_some() { Some(&parent_state) } else { None };
        level.rows(&fine_state, parent, p, step, rows);
        v.get(p).unwrap_or(0) as usize
    }

    pub fn size(&self) -> [usize; 3] {
        self.size
    }

    pub fn labels(&self) -> usize {
        self.n
    }

    /** The coarse site whose cell owns fine site `q` under Cells::Owned (the octree-like partition). */
    pub fn owner(q: [i64; 3]) -> [i64; 3] {
        coarse_cell()
            .iter()
            .filter(|(o, _)| owns(*o))
            .map(|(o, _)| [q[0] - o[0], q[1] - o[1], q[2] - o[2]])
            .find(|c| c.iter().all(|v| v.rem_euclid(2) == 0) && is_site(c.map(|v| v.div_euclid(2))))
            .map(|c| c.map(|v| v.div_euclid(2)))
            .expect("every site has an owner")
    }

    /** The window `at`, size `n` (points), of a volume drawn from `seed`: only what its sites depend on is drawn. */
    pub fn sample(&self, seed: u64, at: [usize; 3], n: [usize; 3]) -> Vol {
        let top = self.options.levels;
        let steps = self.options.steps as i64;
        let reach = |ds: &[[i64; 3]]| ds.iter().flat_map(|d| d.iter().map(|v| v.abs())).max().unwrap_or(0);
        let nr = reach(&self.levels[0].neighbours);
        let pr = if top > 0 { reach(&self.levels[0].parents) } else { 0 };
        type Box = ([i64; 3], [i64; 3]);
        let clip = |b: Box, dims: [usize; 3]| -> Box { ([0, 1, 2].map(|a| b.0[a].max(0)), [0, 1, 2].map(|a| b.1[a].min(dims[a] as i64))) };
        let widen = |b: Box, w: i64| -> Box { (b.0.map(|v| v - w), b.1.map(|v| v + w)) };
        let mut wanted: Vec<Box> = vec![(at.map(|v| v as i64), [0, 1, 2].map(|a| (at[a] + n[a]) as i64))];
        for l in 0..top {
            let first = clip(widen(wanted[l], nr * steps), self.levels[l].dims);
            // Coarse S with 2S within pr of the box.
            let up = ([0, 1, 2].map(|a| (first.0[a] - pr + 1).div_euclid(2)), [0, 1, 2].map(|a| (first.1[a] - 1 + pr).div_euclid(2) + 1));
            wanted.push(clip(up, self.levels[l + 1].dims));
        }
        let mut above: Option<Vol> = None;
        for l in (0..=top).rev() {
            let level = &self.levels[l];
            let mut state = Vol::filled(level.dims, mask(level.n));
            let level_seed = stream(seed, l as u64);
            let parent_n = level.parent_n;
            let parent_state = |q: [i64; 3]| {
                let s = above.as_ref().and_then(|v| v.get(q));
                debug_assert!(s != Some(mask(parent_n)), "a parent read was never drawn");
                s.unwrap_or(parent_n as u16)
            };
            let mut rows = Vec::new();
            let mut pass = Pass::default();
            let mut drawn: Vec<([usize; 3], u16)> = Vec::new();
            for s in 1..=self.options.steps {
                let (lo, hi) = clip(widen(wanted[l], nr * (steps - s as i64)), level.dims);
                drawn.clear();
                for y in lo[1]..hi[1] {
                    for z in lo[2]..hi[2] {
                        for x in lo[0]..hi[0] {
                            if !is_site([x, y, z]) || step_of(unit((hash64(level_seed, x, y, z) >> 32) as u32), self.options.steps) != s {
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
                            level.rows(&fine_state, parent, [x, y, z], s, &mut rows);
                            level.net.forward(&rows, &mut pass);
                            let label = net::draw(&pass.p, hash64(level_seed ^ 0x5a5a, x, y, z));
                            drawn.push(([x, y, z].map(|v| v as usize), label));
                        }
                    }
                }
                for (p, label) in &drawn {
                    state.set(*p, *label);
                }
            }
            // Points not sites back to 0, as in the data; sites not drawn stay masked, so a read of one shows.
            for y in 0..level.dims[1] {
                for z in 0..level.dims[2] {
                    for x in 0..level.dims[0] {
                        if !is_site([x as i64, y as i64, z as i64]) {
                            state.set([x, y, z], 0);
                        }
                    }
                }
            }
            above = Some(state);
        }
        above.expect("level 0").crop(at, n)
    }

    pub fn parameters(&self) -> usize {
        self.levels.iter().map(|l| l.net.parameters()).sum()
    }

    pub fn describe(&self) -> String {
        self.levels
            .iter()
            .enumerate()
            .map(|(l, lv)| {
                let sites = lv.dims.iter().product::<usize>() / 2;
                format!(
                    "L{l}: {} labels, ~{sites} sites, {} neighbours, {} parent offsets ({} rows)",
                    lv.n,
                    lv.neighbours.len(),
                    lv.parents.len(),
                    lv.rows
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_in_one_coarse_cell_two_or_six_and_owned_by_one() {
        let parts = coarse_cell();
        for y in -4..4 {
            for z in -4..4 {
                for x in -4..4 {
                    let q = [x, y, z];
                    if !is_site(q) {
                        continue;
                    }
                    // The coarse cells holding q: parts o with q − o = 2S, S a site.
                    let holders: Vec<([i64; 3], f32)> = parts
                        .iter()
                        .copied()
                        .filter(|(o, _)| {
                            let s2 = [q[0] - o[0], q[1] - o[1], q[2] - o[2]];
                            s2.iter().all(|v| v.rem_euclid(2) == 0) && is_site(s2.map(|v| v.div_euclid(2)))
                        })
                        .collect();
                    assert!([1, 2, 6].contains(&holders.len()), "{q:?} in {} cells", holders.len());
                    assert_eq!(holders.iter().map(|h| h.1).sum::<f32>(), 6.0, "{q:?}: a whole site's weight");
                    assert_eq!(holders.iter().filter(|(o, _)| owns(*o)).count(), 1, "{q:?}: one owner");
                }
            }
        }
    }

    #[test]
    fn a_window_alone_is_that_window_of_the_whole() {
        let n = [33, 9, 33];
        let train: Vec<Vol> = crate::masked::tests::volumes(n, 4).iter().map(crate::scenes::on_lattice).collect();
        let options = Options {
            levels: 2,
            codes: 4,
            neighbours: 2,
            parents: 4,
            steps: 3,
            hidden: 8,
            examples: 20_000,
            epochs: 1,
            height: true,
            augment: false,
            cells: Cells::Shared,
            seed: 1,
        };
        let model = Model::learn(&train, &[], 2, options);
        let (at, w) = ([13, 2, 11], [6, 4, 7]);
        assert_eq!(model.sample(7, at, w), model.sample(7, [0; 3], n).crop(at, w));
    }
}
