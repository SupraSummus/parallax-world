//! Scoring a generator without asking only "how like the data is it":
//! a model that copies its examples is as like them as can be. Three
//! questions instead, each with a number a person need not judge:
//!
//! - Can it be told apart? A critic, a small network (net.rs), learns to
//!   tell windows of real maps from windows of the samples, then is tested
//!   on maps neither it nor the generator saw (a two-sample test by
//!   classifier). Its AUC is 0.5 where it can't, 1 where it always can;
//!   at each scale of window, so it says at what size a model goes wrong,
//!   and the windows it is surest are fake show what to look at. A map
//!   holds few of the largest windows, so the AUC wanders most there:
//!   read it against real maps scored as if a model's.
//! - Is it new? The share of windows of the samples found verbatim among
//!   the training maps' (at any offset), against that share for real maps
//!   held out: above it, the model copies. And verbatim among the model's
//!   other samples: above real maps' share among each other, it repeats
//!   itself (collapses).
//! - Is it laid out as real maps are? Not here: a profile (profile.rs)
//!   says, as distributions, how big each label's parts are and how far
//!   from each other.
//!
//! The network is weak at the third: it sees which labels each block of
//! its window holds, so a street cut every few metres or a house far from
//! any street barely moves its AUC (real maps with their streets in
//! pieces score within a few hundredths of whole ones). Read its AUC
//! against real maps', as a check of what windows hold, and its windows
//! surest fake as where to look.
//!
//! All on label images (image.rs), any labels, flat.

use std::collections::{HashMap, HashSet};

use worldgen::noise::{Rng, stream};

use crate::image::Image;
use crate::net::{self, Net, Pass};
use crate::plan::Context;

/** A critic's window: `side` × `side` blocks of `cell` × `cell` cells, each block seen as which labels it holds, and which most of it. */
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub cell: usize,
    pub side: usize,
}

impl Scale {
    pub fn extent(&self) -> usize {
        self.cell * self.side
    }
}

/** Windows of 16, 64 and 256 cells (metres, for the towns). */
pub const SCALES: [Scale; 3] = [Scale { cell: 1, side: 16 }, Scale { cell: 4, side: 16 }, Scale { cell: 16, side: 16 }];

/** A summed-area table per label of a map. */
struct Sat {
    w: usize,
    n: usize,
    t: Vec<u32>,
}

impl Sat {
    fn of(map: &Image, n: usize) -> Sat {
        let (w, h) = (map.w, map.h);
        let mut t = vec![0u32; (w + 1) * (h + 1) * n];
        for y in 0..h {
            for x in 0..w {
                let own = map.at(x, y) as usize;
                for l in 0..n {
                    let at = |x: usize, y: usize| (y * (w + 1) + x) * n + l;
                    t[at(x + 1, y + 1)] = (own == l) as u32 + t[at(x, y + 1)] + t[at(x + 1, y)] - t[at(x, y)];
                }
            }
        }
        Sat { w, n, t }
    }

    /** Cells of label `l` in the k × k square from (x, y). */
    fn count(&self, x: usize, y: usize, k: usize, l: usize) -> u32 {
        let at = |x: usize, y: usize| self.t[(y * (self.w + 1) + x) * self.n + l];
        at(x + k, y + k) + at(x, y) - at(x, y + k) - at(x + k, y)
    }
}

/** Blocks of a map at every `stride` cells: per label, 0 absent, 1 present, 2 at least half of the block. */
struct Blocks {
    gw: usize,
    gh: usize,
    stride: usize,
    n: usize,
    q: Vec<u8>,
}

impl Blocks {
    fn of(map: &Image, n: usize, cell: usize) -> Blocks {
        let stride = (cell / 2).max(1);
        let (w, h) = (map.w, map.h);
        let sat = Sat::of(map, n);
        let (gw, gh) = ((w - cell) / stride + 1, (h - cell) / stride + 1);
        let mut q = vec![0u8; gw * gh * n];
        for gy in 0..gh {
            for gx in 0..gw {
                for l in 0..n {
                    let c = sat.count(gx * stride, gy * stride, cell, l);
                    q[(gy * gw + gx) * n + l] = if c == 0 {
                        0
                    } else if 2 * c as usize >= cell * cell {
                        2
                    } else {
                        1
                    };
                }
            }
        }
        Blocks { gw, gh, stride, n, q }
    }

    /** Window corners (in blocks) a window of `scale` fits at: 0 ..= the returned, per axis. */
    fn corners(&self, scale: Scale) -> (usize, usize) {
        let k = scale.cell / self.stride;
        let reach = (scale.side - 1) * k;
        (self.gw.saturating_sub(reach + 1), self.gh.saturating_sub(reach + 1))
    }

    fn rows(&self, scale: Scale, gx: usize, gy: usize, rows: &mut Vec<usize>) {
        rows.clear();
        let (k, n) = (scale.cell / self.stride, self.n);
        for j in 0..scale.side {
            for i in 0..scale.side {
                let slot = j * scale.side + i;
                let q = &self.q[((gy + j * k) * self.gw + gx + i * k) * n..][..n];
                for (l, v) in q.iter().enumerate() {
                    let base = (slot * n + l) * 2;
                    if *v >= 1 {
                        rows.push(base);
                    }
                    if *v == 2 {
                        rows.push(base + 1);
                    }
                }
            }
        }
    }
}

/** What the critic of one scale found. */
pub struct Verdict {
    /** Area under the ROC of its scores on test windows: 0.5 can't tell them apart, 1 always can. */
    pub auc: f64,
    /** The test windows of the samples it was surest are fake: (sample, x, y) of their corners, in cells. */
    pub worst: Vec<(usize, usize, usize)>,
    /** Judged against plans: the mean difference of a share between a sample window's plan and its real window's. */
    pub gap: f64,
}

const CRITIC_EXAMPLES: usize = 150_000;
const CRITIC_HIDDEN: usize = 32;
const TEST_WINDOWS: usize = 3000;
const WORST: usize = 8;

/** A window: its map, and its corner in that map's blocks. */
type Window = (usize, usize, usize);

/** A window's context: the mean of its map's over a 4 × 4 grid of points in it (x, y in cells). */
fn window_context(c: &Context, x: usize, y: usize, e: usize) -> Vec<f32> {
    let mut out = vec![0f32; c.dim];
    for j in 0..4 {
        for i in 0..4 {
            let v = c.at((x + e * (2 * i + 1) / 8) as i32, (y + e * (2 * j + 1) / 8) as i32);
            out.iter_mut().zip(v).for_each(|(o, v)| *o += v / 16.0);
        }
    }
    out
}

/** Real windows by their context, quantised, to find one of about a sample window's. */
struct Pool {
    buckets: HashMap<Vec<u8>, Vec<(Window, Vec<f32>)>>,
}

impl Pool {
    const LEVELS: f32 = 4.0;
    const SIZE: usize = 20_000;

    fn key(v: &[f32]) -> Vec<u8> {
        v.iter().map(|x| (x * Self::LEVELS).round().clamp(0.0, Self::LEVELS) as u8).collect()
    }

    fn new(maps: &[Blocks], contexts: &[Context], scale: Scale, seed: u64) -> Pool {
        let mut rng = Rng::new(seed);
        let mut buckets: HashMap<Vec<u8>, Vec<_>> = HashMap::new();
        for _ in 0..Self::SIZE {
            let w = pick(&mut rng, maps, scale);
            let s = maps[w.0].stride;
            let c = window_context(&contexts[w.0], w.1 * s, w.2 * s, scale.extent());
            buckets.entry(Self::key(&c)).or_default().push((w, c));
        }
        Pool { buckets }
    }

    /** A window of the bucket of `want`, or of the nearest bucket holding any; with its context. */
    fn near(&self, want: &[f32], h: u64) -> &(Window, Vec<f32>) {
        let key = Self::key(want);
        let bucket = self.buckets.get(&key).unwrap_or_else(|| {
            let d = |k: &Vec<u8>| k.iter().zip(&key).map(|(a, b)| (*a as i32 - *b as i32).pow(2)).sum::<i32>();
            self.buckets.iter().min_by_key(|(k, _)| (d(k), (*k).clone())).map(|(_, b)| b).expect("real windows")
        });
        &bucket[(h % bucket.len() as u64) as usize]
    }
}

fn pick(rng: &mut Rng, maps: &[Blocks], scale: Scale) -> Window {
    let m = rng.below(maps.len() as u32) as usize;
    let (cx, cy) = maps[m].corners(scale);
    (m, rng.below(cx as u32 + 1) as usize, rng.below(cy as u32 + 1) as usize)
}

/**
 * The real maps critics of one scale learn from (`real`) and are tested
 * on (`held`, maps the generators never saw), made once for every model
 * judged. Given their contexts, it can judge samples against the plans
 * they were drawn to.
 */
pub struct Reference {
    scale: Scale,
    n: usize,
    real: Vec<Blocks>,
    held: Vec<Blocks>,
    pools: Option<(Pool, Pool)>,
}

impl Reference {
    pub fn new(real: &[Image], held: &[Image], n: usize, scale: Scale, contexts: Option<(&[Context], &[Context])>) -> Reference {
        let blocks = |maps: &[Image]| -> Vec<Blocks> { maps.iter().map(|m| Blocks::of(m, n, scale.cell)).collect() };
        let (real, held) = (blocks(real), blocks(held));
        let pools = contexts.map(|(r, h)| (Pool::new(&real, r, scale, 1), Pool::new(&held, h, scale, 2)));
        Reference { scale, n, real, held, pools }
    }

    /**
     * A critic trained on windows of the real maps against those of the
     * even samples, tested on windows of the held maps against the odd
     * ones. With `plans` (one per sample), each sample window is set
     * against a real one of about its plan's context, so the critic judges
     * how a plan was filled in, not whether real places have such plans:
     * a painted plan of town and forest only would otherwise be told apart
     * by the fields it lacks.
     */
    pub fn judge(&self, samples: &[Image], plans: Option<&[Context]>, seed: u64) -> Verdict {
        let (train, test): (Vec<usize>, Vec<usize>) = (0..samples.len()).partition(|i| i % 2 == 0);
        assert!(!test.is_empty(), "samples to train the critic on and to test it with");
        let (scale, e) = (self.scale, self.scale.extent());
        let blocks = |idx: &[usize]| -> Vec<Blocks> { idx.iter().map(|i| Blocks::of(&samples[*i], self.n, scale.cell)).collect() };
        let (fake_train, fake_test) = (blocks(&train), blocks(&test));
        let mut gaps = Vec::new();
        let pools = self.pools.as_ref();
        let mut pairs =
            |fakes: &[Blocks], idx: &[usize], reals: &[Blocks], pool: Option<&Pool>, k: usize, salt: u64| -> Vec<(Window, Window)> {
                let mut rng = Rng::new(stream(seed, salt));
                (0..k)
                    .map(|i| {
                        let f = pick(&mut rng, fakes, scale);
                        let r = match plans {
                            Some(plans) => {
                                let s = fakes[f.0].stride;
                                let want = window_context(&plans[idx[f.0]], f.1 * s, f.2 * s, e);
                                let (r, c) =
                                    pool.expect("the real maps' contexts, to match plans").near(&want, stream(seed ^ salt, i as u64));
                                gaps.push(want.iter().zip(c).map(|(a, b)| (a - b).abs() as f64).sum::<f64>() / want.len() as f64);
                                *r
                            }
                            None => pick(&mut rng, reals, scale),
                        };
                        (f, r)
                    })
                    .collect()
            };
        let train_pairs = pairs(&fake_train, &train, &self.real, pools.map(|p| &p.0), CRITIC_EXAMPLES / 2, 2);
        let test_pairs = pairs(&fake_test, &test, &self.held, pools.map(|p| &p.1), TEST_WINDOWS, 3);
        let gap = gaps.iter().sum::<f64>() / gaps.len().max(1) as f64;

        let mut net = Net::new(scale.side * scale.side * self.n * 2, CRITIC_HIDDEN, 2, stream(seed, 1));
        let mut rng = Rng::new(stream(seed, 4));
        net::train(&mut net, CRITIC_EXAMPLES, &mut |rows| {
            let ((fm, fx, fy), (rm, rx, ry)) = train_pairs[rng.below(train_pairs.len() as u32) as usize];
            let fake = rng.below(2) as usize;
            if fake == 1 {
                fake_train[fm].rows(scale, fx, fy, rows)
            } else {
                self.real[rm].rows(scale, rx, ry, rows)
            }
            fake
        });
        let (mut pass, mut rows) = (Pass::default(), Vec::new());
        let mut score = |b: &Blocks, x: usize, y: usize| {
            b.rows(scale, x, y, &mut rows);
            net.forward(&rows, &mut pass);
            pass.p[1]
        };
        let real_s: Vec<f32> = test_pairs.iter().map(|(_, (m, x, y))| score(&self.held[*m], *x, *y)).collect();
        let mut fake_s: Vec<(f32, Window)> =
            test_pairs.iter().map(|((m, x, y), _)| (score(&fake_test[*m], *x, *y), (*m, *x, *y))).collect();
        let auc = auc(&real_s, &fake_s.iter().map(|s| s.0).collect::<Vec<_>>());
        fake_s.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut worst: Vec<Window> = Vec::new();
        for (_, (m, x, y)) in fake_s {
            let (m, x, y) = (test[m], x * fake_test[0].stride, y * fake_test[0].stride);
            // Not overlapping one already kept, so they show different things.
            if !worst.iter().any(|(wm, wx, wy)| *wm == m && wx.abs_diff(x) < e && wy.abs_diff(y) < e) {
                worst.push((m, x, y));
            }
            if worst.len() == WORST {
                break;
            }
        }
        Verdict { auc, worst, gap }
    }
}

/** The chance a positive's score is above a negative's (ties half). */
pub fn auc(negatives: &[f32], positives: &[f32]) -> f64 {
    let mut all: Vec<(f32, bool)> = negatives.iter().map(|s| (*s, false)).chain(positives.iter().map(|s| (*s, true))).collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Sum of the positives' ranks, ties given their mean rank.
    let (mut i, mut ranks) = (0, 0.0);
    while i < all.len() {
        let mut j = i;
        while j < all.len() && all[j].0 == all[i].0 {
            j += 1;
        }
        let mean = (i + j + 1) as f64 / 2.0;
        ranks += all[i..j].iter().filter(|a| a.1).count() as f64 * mean;
        i = j;
    }
    let (p, q) = (positives.len() as f64, negatives.len() as f64);
    (ranks - p * (p + 1.0) / 2.0) / (p * q).max(1.0)
}

/**
 * Hashes of a map's k × k windows (every offset) that are busy: no label
 * over BUSY of them. Uniform windows (all forest) match anything of the
 * same label, and say nothing of copying.
 */
pub fn windows(map: &Image, k: usize, step: usize) -> Vec<u64> {
    const BUSY: f64 = 0.8;
    const B: u64 = 0x9e37_79b9_7f4a_7c15;
    const C: u64 = 0xc2b2_ae3d_27d4_eb4f;
    let (w, h) = (map.w, map.h);
    if w < k || h < k {
        return Vec::new();
    }
    let n = map.labels();
    // Row hashes of k cells, then columns of k row hashes; counts per label by a summed-area table.
    let mut row = vec![0u64; (w - k + 1) * h];
    for y in 0..h {
        for x in 0..=w - k {
            let mut v = 0u64;
            for i in 0..k {
                v = v.wrapping_mul(B).wrapping_add(map.at(x + i, y) as u64 + 1);
            }
            row[y * (w - k + 1) + x] = v;
        }
    }
    let sat = Sat::of(map, n);
    let limit = (BUSY * (k * k) as f64) as u32;
    let mut out = Vec::new();
    for y in (0..=h - k).step_by(step) {
        for x in (0..=w - k).step_by(step) {
            let busy = (0..n).all(|l| sat.count(x, y, k, l) <= limit);
            if !busy {
                continue;
            }
            let mut v = 0u64;
            for j in 0..k {
                v = v.wrapping_mul(C).wrapping_add(row[(y + j) * (w - k + 1) + x]);
            }
            out.push(v);
        }
    }
    out
}

/** Busy k × k windows of `maps`, every offset, as a set. */
pub fn window_set(maps: &[Image], k: usize) -> HashSet<u64> {
    maps.iter().flat_map(|m| windows(m, k, 1)).collect()
}

/** The share of the busy k × k windows of `maps` (every `step` cells) in `set`. */
pub fn found(maps: &[Image], k: usize, set: &HashSet<u64>, step: usize) -> f64 {
    let (mut hit, mut all) = (0usize, 0usize);
    for m in maps {
        for v in windows(m, k, step) {
            all += 1;
            hit += set.contains(&v) as usize;
        }
    }
    hit as f64 / all.max(1) as f64
}

/** The share of busy k × k windows of each map found in the other maps': how much maps repeat each other. */
pub fn repeated(maps: &[Image], k: usize, step: usize) -> f64 {
    let sets: Vec<HashSet<u64>> = maps.iter().map(|m| windows(m, k, 1).into_iter().collect()).collect();
    let (mut hit, mut all) = (0usize, 0usize);
    for (i, m) in maps.iter().enumerate() {
        for v in windows(m, k, step) {
            all += 1;
            hit += sets.iter().enumerate().any(|(j, s)| j != i && s.contains(&v)) as usize;
        }
    }
    hit as f64 / all.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auc_of_separated_and_of_mixed_scores() {
        assert_eq!(auc(&[0.1, 0.2], &[0.8, 0.9]), 1.0);
        assert_eq!(auc(&[0.8, 0.9], &[0.1, 0.2]), 0.0);
        assert_eq!(auc(&[0.5, 0.5], &[0.5, 0.5]), 0.5);
    }

    #[test]
    fn a_copied_window_is_found_and_a_new_one_is_not() {
        let mut rng = Rng::new(3);
        let map = Image::new(40, 40, (0..1600).map(|_| rng.below(4) as u16).collect());
        let set = window_set(std::slice::from_ref(&map), 8);
        let copy = map.crop(5, 7, 20, 20);
        assert_eq!(found(&[copy], 8, &set, 1), 1.0);
        let other = Image::new(20, 20, (0..400).map(|_| rng.below(4) as u16).collect());
        assert_eq!(found(&[other], 8, &set, 1), 0.0);
    }

    #[test]
    fn a_critic_tells_noise_from_stripes_and_not_stripes_from_stripes() {
        let stripes = |s: u64| -> Image {
            let off = (s % 7) as usize;
            Image::new(64, 64, (0..64 * 64).map(|i| (((i % 64 + off) / 4) % 2) as u16).collect())
        };
        let mut rng = Rng::new(5);
        let noise: Vec<Image> = (0..4).map(|_| Image::new(64, 64, (0..64 * 64).map(|_| rng.below(2) as u16).collect())).collect();
        let real: Vec<Image> = (0..4).map(stripes).collect();
        let held: Vec<Image> = (10..12).map(stripes).collect();
        let reference = Reference::new(&real, &held, 2, Scale { cell: 1, side: 8 }, None);
        assert!(reference.judge(&noise, None, 1).auc > 0.95);
        let more: Vec<Image> = (20..24).map(stripes).collect();
        assert!(reference.judge(&more, None, 1).auc < 0.7);
    }
}
