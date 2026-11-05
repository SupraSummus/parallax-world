//! Synthesis by coordinates (exemplar.rs) on the FCC lattice
//! (masked_fcc.rs's volumes).
//!
//! Children follow the engine's coarse cells: a site in two cells or six
//! is offered an address by each parent, and the correction picks the one
//! that fits, where an octree would have to cut. Correction runs one class
//! of sites at a time (by which coordinates are odd), so no site reads a
//! neighbour changed in the same pass. `height` keeps addresses near the
//! site's own height, so ground and sky stay where they are.

use worldgen::noise::{Rng, hash64, stream, unit};

use crate::exemplar::reflect;
use crate::masked_fcc::{self, Model as Fcc, is_site};
use crate::net;
use crate::vol::Vol;

#[derive(Clone, Debug)]
pub struct Options {
    pub levels: usize,
    pub passes: usize,
    /** Per level, coarsest first, in the level's spacings. */
    pub jitter: Vec<f64>,
    pub clusters: usize,
    pub pool: usize,
    /** Weight of a candidate's height from the site's, in boxes' heights squared. */
    pub height: f64,
    /** Whether the top level starts from an example per site rather than one for all. */
    pub mixed: bool,
    pub seed: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Coord {
    pub e: u16,
    pub p: [i32; 3],
}

const NEAREST: [[i32; 3]; 12] = [
    [1, 1, 0],
    [1, 0, 1],
    [0, 1, 1],
    [1, -1, 0],
    [1, 0, -1],
    [0, 1, -1],
    [-1, -1, 0],
    [-1, 0, -1],
    [0, -1, -1],
    [-1, 1, 0],
    [-1, 0, 1],
    [0, -1, 1],
];
const CELLS: usize = 13;
/** What a site holds until drawn, so that reading one too early shows (in debug builds). */
const UNDRAWN: Coord = Coord { e: u16::MAX, p: [0; 3] };

/** The engine's coarse cell (masked_fcc.rs), offsets only. */
fn coarse_cell() -> Vec<[i32; 3]> {
    masked_fcc::coarse_cell().into_iter().map(|(o, _)| o.map(|v| v as i32)).collect()
}

pub struct Exemplars {
    pub options: Options,
    n: usize,
    size: [usize; 3],
    labels: Vec<Vol>,
    /** Per example, per label, counts of sites in [0, x) × [0, y) × [0, z), wrapping. */
    sat: Vec<Vec<u16>>,
    levels: Vec<(Vec<f32>, Vec<Vec<Coord>>)>,
}

impl Exemplars {
    pub fn new(examples: &[Vol], n: usize, options: Options) -> Exemplars {
        let size = examples[0].n;
        let top = options.levels;
        assert!(examples.iter().all(|v| v.n == size), "examples of one size");
        assert!(size.iter().all(|s| (s - 1).is_multiple_of(1 << top)), "whole cells at the top");
        // The tables wrap at 2^16: a box of the top level must hold fewer sites.
        assert!((1usize << (3 * top)) / 2 < 1 << 16, "boxes small enough for the tables");
        let s1 = size.map(|s| s + 1);
        let sat = examples
            .iter()
            .map(|v| {
                let mut t = vec![0u16; s1[0] * s1[1] * s1[2] * n];
                let at = |x: usize, y: usize, z: usize, l: usize| ((y * s1[2] + z) * s1[0] + x) * n + l;
                for y in 0..size[1] {
                    for z in 0..size[2] {
                        for x in 0..size[0] {
                            let site = (x + y + z) % 2 == 0;
                            for l in 0..n {
                                let own = (site && v.at([x, y, z]) as usize == l) as u16;
                                let s = own
                                    .wrapping_add(t[at(x, y + 1, z + 1, l)])
                                    .wrapping_add(t[at(x + 1, y, z + 1, l)])
                                    .wrapping_add(t[at(x + 1, y + 1, z, l)])
                                    .wrapping_sub(t[at(x, y, z + 1, l)])
                                    .wrapping_sub(t[at(x, y + 1, z, l)])
                                    .wrapping_sub(t[at(x + 1, y, z, l)])
                                    .wrapping_add(t[at(x, y, z, l)]);
                                t[at(x + 1, y + 1, z + 1, l)] = s;
                            }
                        }
                    }
                }
                t
            })
            .collect();
        let mut ex = Exemplars { options: options.clone(), n, size, labels: examples.to_vec(), sat, levels: Vec::new() };
        let dim = CELLS * n;
        for l in 0..=top {
            if !ex.pools_at(l) {
                ex.levels.push((Vec::new(), Vec::new()));
                continue;
            }
            let mut rng = Rng::new(stream(options.seed, 50 + l as u64));
            const SAMPLE: usize = 30_000;
            let coords: Vec<Coord> = (0..SAMPLE)
                .map(|_| {
                    ex.on_site(Coord { e: rng.below(examples.len() as u32) as u16, p: [0, 1, 2].map(|a| rng.below(size[a] as u32) as i32) })
                })
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

    /** Whether level `l` draws candidates from pools: not the two finest. */
    fn pools_at(&self, l: usize) -> bool {
        l + 2 <= self.options.levels && self.options.pool > 0
    }

    fn spacing(&self, level: usize) -> i32 {
        1 << (self.options.levels - level)
    }

    /** A coordinate inside its example and on a site (moved by one along x if not). */
    fn on_site(&self, c: Coord) -> Coord {
        let mut p = [0, 1, 2].map(|a| reflect(c.p[a], self.size[a]));
        if (p[0] + p[1] + p[2]).rem_euclid(2) == 1 {
            p[0] += if p[0] > 0 { -1 } else { 1 };
        }
        Coord { e: c.e, p }
    }

    /** The √ shares of the labels among the sites of the box of side h about `c` (clipped to the example). */
    fn shares(&self, level: usize, c: Coord, out: &mut [f32]) {
        let h = self.spacing(level);
        let s1 = self.size.map(|s| s + 1);
        let lo = [0, 1, 2].map(|a| (c.p[a] - h / 2).clamp(0, self.size[a] as i32 - 1) as usize);
        let hi = [0, 1, 2].map(|a| (lo[a] as i32 + h.max(1)).min(self.size[a] as i32) as usize);
        let t = &self.sat[c.e as usize];
        let n = self.n;
        let at = |x: usize, y: usize, z: usize, l: usize| t[((y * s1[2] + z) * s1[0] + x) * n + l];
        let mut total = 0.0f32;
        for (l, o) in out.iter_mut().enumerate().take(n) {
            let v = at(hi[0], hi[1], hi[2], l)
                .wrapping_sub(at(lo[0], hi[1], hi[2], l))
                .wrapping_sub(at(hi[0], lo[1], hi[2], l))
                .wrapping_sub(at(hi[0], hi[1], lo[2], l))
                .wrapping_add(at(lo[0], lo[1], hi[2], l))
                .wrapping_add(at(lo[0], hi[1], lo[2], l))
                .wrapping_add(at(hi[0], lo[1], lo[2], l))
                .wrapping_sub(at(lo[0], lo[1], lo[2], l));
            *o = v as f32;
            total += v as f32;
        }
        out.iter_mut().take(n).for_each(|o| *o = (*o / total.max(1.0)).sqrt());
    }

    fn neighbourhood(&self, level: usize, c: Coord, out: &mut [f32]) {
        let h = self.spacing(level);
        let n = self.n;
        self.shares(level, c, &mut out[..n]);
        for (k, d) in NEAREST.iter().enumerate() {
            self.shares(level, Coord { e: c.e, p: [0, 1, 2].map(|a| c.p[a] + h * d[a]) }, &mut out[(k + 1) * n..][..n]);
        }
    }

    pub fn label(&self, c: Coord) -> u16 {
        self.labels[c.e as usize].at(c.p.map(|v| v as usize))
    }

    /** The window `at`, size `w` (points), of a volume the examples' size drawn from `seed`; points not sites 0. */
    pub fn sample(&self, seed: u64, at: [usize; 3], w: [usize; 3]) -> (Vol, Vec<Coord>) {
        let top = self.options.levels;
        let subpasses = 4 * self.options.passes as i64;
        let dims = |l: usize| self.size.map(|s| ((s - 1) >> (top - l)) as i64 + 1);
        type Rect = ([i64; 3], [i64; 3]);
        let clip = |r: Rect, d: [i64; 3]| -> Rect { ([0, 1, 2].map(|a| r.0[a].max(0)), [0, 1, 2].map(|a| r.1[a].min(d[a]))) };
        let widen = |r: Rect, m: i64| -> Rect { (r.0.map(|v| v - m), r.1.map(|v| v + m)) };
        let mut wanted: Vec<Rect> = vec![([0; 3], [0; 3]); top + 1];
        wanted[top] = (at.map(|v| v as i64), [0, 1, 2].map(|a| (at[a] + w[a]) as i64));
        for l in (1..=top).rev() {
            let r = clip(widen(wanted[l], subpasses), dims(l));
            // Parents S of q = 2S + o, |o| ≤ 2 on each axis.
            wanted[l - 1] =
                clip(([0, 1, 2].map(|a| (r.0[a] - 2).div_euclid(2)), [0, 1, 2].map(|a| (r.1[a] + 1).div_euclid(2) + 1)), dims(l - 1));
        }
        let parts = coarse_cell();
        let n = self.n;
        let dim = CELLS * n;
        let (mut f, mut g) = (vec![0f32; dim], vec![0f32; dim]);
        let first = (hash64(seed, 0, 0, 7) % self.labels.len() as u64) as u16;
        let mut above: Vec<Coord> = Vec::new();
        let mut above_d = [0i64; 3];
        let class = |p: [i64; 3]| (p[0].rem_euclid(2), p[1].rem_euclid(2), p[2].rem_euclid(2));
        for (l, &want) in wanted.iter().enumerate() {
            let d = dims(l);
            let h = self.spacing(l);
            let level_seed = stream(seed, l as u64);
            let idx = |p: [i64; 3]| ((p[1] * d[2] + p[2]) * d[0] + p[0]) as usize;
            let up_idx = |s: [i64; 3]| ((s[1] * above_d[2] + s[2]) * above_d[0] + s[0]) as usize;
            let mut state = vec![UNDRAWN; (d[0] * d[1] * d[2]) as usize];
            // The coordinates each parent offers a site, the owner's first.
            let offers = |q: [i64; 3]| -> Vec<Coord> {
                if l == 0 {
                    let e =
                        if self.options.mixed { (hash64(seed, q[0], q[1], q[2] * 8 + 3) % self.labels.len() as u64) as u16 } else { first };
                    return vec![self.on_site(Coord { e, p: q.map(|v| (v * h as i64) as i32) })];
                }
                let owner = Fcc::owner(q);
                let mut out = Vec::new();
                for o in &parts {
                    let s2 = [q[0] - o[0] as i64, q[1] - o[1] as i64, q[2] - o[2] as i64];
                    if s2.iter().any(|v| v.rem_euclid(2) != 0) {
                        continue;
                    }
                    let s = s2.map(|v| v.div_euclid(2));
                    if !is_site(s) || (0..3).any(|a| s[a] < 0 || s[a] >= above_d[a]) {
                        continue;
                    }
                    let c = above[up_idx(s)];
                    debug_assert!(c != UNDRAWN, "a parent read was never drawn");
                    let offer = self.on_site(Coord { e: c.e, p: [0, 1, 2].map(|a| c.p[a] + h * o[a]) });
                    if s == owner { out.insert(0, offer) } else { out.push(offer) }
                }
                out
            };
            let r0 = clip(widen(want, subpasses), d);
            let jitter = self.options.jitter.get(l).copied().unwrap_or(0.0);
            for y in r0.0[1]..r0.1[1] {
                for z in r0.0[2]..r0.1[2] {
                    for x in r0.0[0]..r0.1[0] {
                        let q = [x, y, z];
                        if !is_site(q) {
                            continue;
                        }
                        let Some(base) = offers(q).first().copied() else { continue };
                        let hj = hash64(level_seed, x, y, z);
                        let j = |b: u64| ((unit((b & 0xffff) as u32 * 0x10001) * 2.0 - 1.0) * jitter * h as f64).round() as i32;
                        state[idx(q)] =
                            self.on_site(Coord { e: base.e, p: [base.p[0] + j(hj), base.p[1] + j(hj >> 16), base.p[2] + j(hj >> 32)] });
                    }
                }
            }
            let mut sub = 0;
            for pass in 0..self.options.passes {
                for cls in [(0, 0, 0), (1, 1, 0), (1, 0, 1), (0, 1, 1)] {
                    sub += 1;
                    let r = clip(widen(want, subpasses - sub), d);
                    for y in r.0[1]..r.1[1] {
                        for z in r.0[2]..r.1[2] {
                            for x in r.0[0]..r.1[0] {
                                let q = [x, y, z];
                                if !is_site(q) || class(q) != cls {
                                    continue;
                                }
                                let mut have = [false; CELLS];
                                let mut cells: [[i64; 3]; CELLS] = [q; CELLS];
                                for (k, dd) in NEAREST.iter().enumerate() {
                                    cells[k + 1] = [x + dd[0] as i64, y + dd[1] as i64, z + dd[2] as i64];
                                }
                                for (k, c) in cells.iter().enumerate() {
                                    if (0..3).all(|a| c[a] >= 0 && c[a] < d[a]) {
                                        have[k] = true;
                                        debug_assert!(state[idx(*c)] != UNDRAWN, "a neighbour read was never drawn");
                                        self.shares(l, state[idx(*c)], &mut f[k * n..][..n]);
                                    }
                                }
                                let mut candidates = vec![state[idx(q)]];
                                candidates.extend(offers(q));
                                for (k, dd) in NEAREST.iter().enumerate() {
                                    if have[k + 1] {
                                        let c = state[idx(cells[k + 1])];
                                        candidates.push(self.on_site(Coord { e: c.e, p: [0, 1, 2].map(|a| c.p[a] - h * dd[a]) }));
                                    }
                                }
                                if self.pools_at(l) {
                                    let centre = f[..n].to_vec();
                                    for k in (0..CELLS).filter(|k| !have[*k]) {
                                        f[k * n..][..n].copy_from_slice(&centre);
                                    }
                                    let (centroids, pools) = &self.levels[l];
                                    let pool = &pools[net::nearest(centroids, dim, &f)];
                                    for i in 0..self.options.pool.min(pool.len()) {
                                        let hh = hash64(level_seed ^ (pass as u64 + 1), x, y, z * 64 + i as i64);
                                        candidates.push(pool[(hh % pool.len() as u64) as usize]);
                                    }
                                }
                                let place_y = (y * h as i64) as f64;
                                let mut best = (f64::INFINITY, candidates[0]);
                                for c in candidates {
                                    self.neighbourhood(l, c, &mut g);
                                    let mut cost = 0.0f64;
                                    for k in (0..CELLS).filter(|k| have[*k]) {
                                        cost += f[k * n..][..n]
                                            .iter()
                                            .zip(&g[k * n..][..n])
                                            .map(|(a, b)| ((a - b) * (a - b)) as f64)
                                            .sum::<f64>();
                                    }
                                    cost += self.options.height * ((c.p[1] as f64 - place_y) / self.size[1] as f64).powi(2);
                                    if cost < best.0 {
                                        best = (cost, c);
                                    }
                                }
                                state[idx(q)] = best.1;
                            }
                        }
                    }
                }
            }
            above = state;
            above_d = d;
        }
        let mut out = Vol::filled(w, 0);
        let mut coords = Vec::new();
        for y in 0..w[1] {
            for z in 0..w[2] {
                for x in 0..w[0] {
                    let q = [(x + at[0]) as i64, (y + at[1]) as i64, (z + at[2]) as i64];
                    if is_site(q) {
                        let c = above[((q[1] * above_d[2] + q[2]) * above_d[0] + q[0]) as usize];
                        out.set([x, y, z], self.label(c));
                        coords.push(c);
                    }
                }
            }
        }
        (out, coords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_alone_is_that_window_of_the_whole() {
        let n = [33, 9, 33];
        let examples: Vec<Vol> = crate::masked::tests::volumes(n, 4).iter().map(crate::scenes::on_lattice).collect();
        let options =
            Options { levels: 2, passes: 1, jitter: vec![0.5, 0.3, 0.0], clusters: 4, pool: 2, height: 1.0, mixed: true, seed: 1 };
        let ex = Exemplars::new(&examples, 2, options);
        let (at, w) = ([13, 2, 11], [6, 4, 7]);
        let (alone, whole) = (ex.sample(7, at, w).1, ex.sample(7, [0; 3], n).1);
        // The whole's coordinates of the window's sites, in the order sample gives them (y, z, x).
        let mut window = Vec::new();
        let mut i = 0;
        for y in 0..n[1] {
            for z in 0..n[2] {
                for x in 0..n[0] {
                    if is_site([x as i64, y as i64, z as i64]) {
                        if (0..3).all(|a| [x, y, z][a] >= at[a] && [x, y, z][a] < at[a] + w[a]) {
                            window.push(whole[i]);
                        }
                        i += 1;
                    }
                }
            }
        }
        assert_eq!(alone, window);
    }
}
