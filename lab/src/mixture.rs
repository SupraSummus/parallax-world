//! A mixture of adjacency models fitted to the blocks of a label image:
//! each component is a rule-set, each block's component its biome.
//!
//! A block is a bag of tokens: its neighbour pairs (left–right and
//! bottom–top, with the labels in order) or its overlapping 2 × 2
//! patterns. A component is a multinomial over tokens, so its likelihood of
//! a block is a composite likelihood of the block's adjacencies, the
//! pseudo-likelihood of a pairwise random field, or of an overlapping-WFC
//! model with N = 2. Tokens overlap (each cell is in about four pairs, or
//! four patterns), so their log-likelihood is weighted down by as much.
//!
//! Fitted by classification EM: blocks go to the component that explains
//! them best, components are re-estimated from their blocks. Started from
//! k-means++ on the blocks' label histograms. The number of components is
//! chosen by description length: the blocks' tokens and biomes coded with
//! the model, plus half a log of the data per free parameter (BIC as MDL).

use std::collections::HashMap;

use worldgen::noise::{Rng, hash64};

use crate::image::Image;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tiles {
    /** Neighbour pairs of single labels. */
    Labels,
    /** Overlapping 2 × 2 patterns. */
    Patterns,
    /** Single labels, no adjacency: a block's histogram, as a topic model sees a document. */
    Histogram,
}

/** An image's blocks as bags of tokens. */
pub struct Blocks {
    /** Blocks across and up; block size in cells. */
    pub nx: usize,
    pub ny: usize,
    pub b: usize,
    /** Per block, its (token, count), tokens sorted. */
    pub tokens: Vec<Vec<(u32, u32)>>,
    /** Per block, its labels' shares. */
    pub histogram: Vec<Vec<f64>>,
    /** Distinct tokens. */
    pub vocabulary: usize,
    /** The weight of a token's log-likelihood: how much of a cell's information it holds. */
    pub weight: f64,
}

/** The whole b × b blocks of `image`, as tokens of `tiles`; patterns are numbered in the order met, in `dictionary`. */
pub fn blocks(image: &Image, b: usize, tiles: Tiles, dictionary: &mut HashMap<u64, u32>) -> Blocks {
    blocks_of(image, image.labels(), b, tiles, dictionary)
}

/** As `blocks`, of an image of (at most) `labels` labels: blocks of images with the same labels number their tokens alike. */
pub fn blocks_of(image: &Image, labels: usize, b: usize, tiles: Tiles, dictionary: &mut HashMap<u64, u32>) -> Blocks {
    let (nx, ny, labels) = (image.w / b, image.h / b, labels.max(image.labels()).max(1));
    let l = labels as u64;
    let mut tokens = Vec::with_capacity(nx * ny);
    let mut histogram = Vec::with_capacity(nx * ny);
    let mut bag: HashMap<u32, u32> = HashMap::new();
    for by in 0..ny {
        for bx in 0..nx {
            bag.clear();
            let mut h = vec![0.0; labels];
            let (x0, y0) = (bx * b, by * b);
            for y in y0..y0 + b {
                for x in x0..x0 + b {
                    let a = image.at(x, y) as u64;
                    h[a as usize] += 1.0 / (b * b) as f64;
                    let mut add = |t: u64| *bag.entry(t as u32).or_insert(0) += 1;
                    match tiles {
                        Tiles::Labels => {
                            if x + 1 < x0 + b {
                                add(a * l + image.at(x + 1, y) as u64);
                            }
                            if y + 1 < y0 + b {
                                add(l * l + a * l + image.at(x, y + 1) as u64);
                            }
                        }
                        Tiles::Histogram => add(a),
                        Tiles::Patterns => {
                            if x + 1 < x0 + b && y + 1 < y0 + b {
                                let code = ((a * l + image.at(x + 1, y) as u64) * l + image.at(x, y + 1) as u64) * l
                                    + image.at(x + 1, y + 1) as u64;
                                let next = dictionary.len() as u32;
                                add(*dictionary.entry(code).or_insert(next) as u64);
                            }
                        }
                    }
                }
            }
            let mut bag: Vec<(u32, u32)> = bag.iter().map(|(t, c)| (*t, *c)).collect();
            bag.sort_unstable();
            tokens.push(bag);
            histogram.push(h);
        }
    }
    let (vocabulary, weight) = match tiles {
        Tiles::Labels => (2 * labels * labels, 0.5),
        Tiles::Patterns => (dictionary.len(), 0.25),
        Tiles::Histogram => (labels, 1.0),
    };
    Blocks { nx, ny, b, tokens, histogram, vocabulary, weight }
}

/** A fitted mixture. */
#[derive(Clone)]
pub struct Fit {
    pub m: usize,
    /** Per block, its component. */
    pub assign: Vec<u16>,
    /** Per component, log θ of each token (dense), of a token it never saw, and log π. */
    pub log_theta: Vec<Vec<f64>>,
    pub log_unseen: Vec<f64>,
    pub log_pi: Vec<f64>,
    /** Description length in bits: the data given the model, and the model. */
    pub data_bits: f64,
    pub model_bits: f64,
}

impl Fit {
    pub fn bits(&self) -> f64 {
        self.data_bits + self.model_bits
    }

    /** The assignment as a label image, a cell per block. */
    pub fn image(&self, blocks: &Blocks) -> Image {
        Image::new(blocks.nx, blocks.ny, self.assign.clone())
    }

    /** Each block of `blocks` (tokens numbered as when fitted) to its likeliest component. */
    pub fn classify(&self, blocks: &Blocks) -> Vec<u16> {
        blocks.tokens.iter().map(|t| best(self, t, blocks.weight).0).collect()
    }
}

/** Additive smoothing of each component's token counts. */
const ALPHA: f64 = 0.1;

fn score(fit: &Fit, m: usize, tokens: &[(u32, u32)], weight: f64) -> f64 {
    let (theta, unseen) = (&fit.log_theta[m], fit.log_unseen[m]);
    fit.log_pi[m] + weight * tokens.iter().map(|(t, c)| *c as f64 * theta.get(*t as usize).copied().unwrap_or(unseen)).sum::<f64>()
}

fn best(fit: &Fit, tokens: &[(u32, u32)], weight: f64) -> (u16, f64) {
    (0..fit.m).map(|m| (m as u16, score(fit, m, tokens, weight))).fold((0, f64::MIN), |a, b| if b.1 > a.1 { b } else { a })
}

/** The components' parameters from an assignment, and its description length. */
fn estimate(blocks: &Blocks, m: usize, assign: Vec<u16>) -> Fit {
    let v = blocks.vocabulary.max(1);
    let mut counts = vec![vec![0.0; v]; m];
    let mut size = vec![0usize; m];
    for (bag, z) in blocks.tokens.iter().zip(&assign) {
        size[*z as usize] += 1;
        for (t, c) in bag {
            counts[*z as usize][*t as usize] += *c as f64;
        }
    }
    let n = assign.len() as f64;
    let log_pi: Vec<f64> = size.iter().map(|s| ((*s as f64 + 0.5) / (n + 0.5 * m as f64)).ln()).collect();
    let mut model_bits = 0.5 * (m as f64 - 1.0) * n.log2();
    let log_unseen = counts.iter().map(|c| (ALPHA / (c.iter().sum::<f64>() + ALPHA * v as f64)).ln()).collect();
    let log_theta = counts
        .iter()
        .map(|c| {
            let total: f64 = c.iter().sum();
            let free = c.iter().filter(|x| **x > 0.0).count() as f64;
            model_bits += 0.5 * free * (blocks.weight * total).max(2.0).log2();
            c.iter().map(|x| ((x + ALPHA) / (total + ALPHA * v as f64)).ln()).collect()
        })
        .collect();
    let mut fit = Fit { m, assign, log_theta, log_unseen, log_pi, data_bits: 0.0, model_bits };
    let nats: f64 = blocks.tokens.iter().zip(&fit.assign).map(|(t, z)| score(&fit, *z as usize, t, blocks.weight)).sum();
    fit.data_bits = -nats / std::f64::consts::LN_2;
    fit
}

/** k-means++ on the label histograms, then Lloyd's iterations: the starting assignment. */
fn kmeans(blocks: &Blocks, m: usize, seed: u64) -> Vec<u16> {
    let h = &blocks.histogram;
    let dist = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>();
    let mut rng = Rng::new(hash64(seed, m as i64, 0, 0));
    let mut centres = vec![h[rng.below(h.len() as u32) as usize].clone()];
    while centres.len() < m {
        let d: Vec<f64> = h.iter().map(|x| centres.iter().map(|c| dist(x, c)).fold(f64::MAX, f64::min)).collect();
        let total: f64 = d.iter().sum();
        let mut pick = rng.next() * total;
        let i = d.iter().position(|x| {
            pick -= x;
            pick <= 0.0
        });
        centres.push(h[i.unwrap_or(h.len() - 1)].clone());
    }
    let mut assign = vec![0u16; h.len()];
    for _ in 0..15 {
        for (z, x) in assign.iter_mut().zip(h) {
            *z = (0..m).min_by(|a, b| dist(x, &centres[*a]).total_cmp(&dist(x, &centres[*b]))).unwrap_or(0) as u16;
        }
        for (c, centre) in centres.iter_mut().enumerate() {
            let members: Vec<&Vec<f64>> = h.iter().zip(&assign).filter(|(_, z)| **z as usize == c).map(|(x, _)| x).collect();
            if !members.is_empty() {
                for (i, v) in centre.iter_mut().enumerate() {
                    *v = members.iter().map(|x| x[i]).sum::<f64>() / members.len() as f64;
                }
            }
        }
    }
    assign
}

/**
 * A mixture of `m` components fitted to `blocks` by classification EM, the
 * best of `restarts` starts. With `smooth` above 0, a block's biome also
 * scores that many nats per side it shares with its neighbours' (a Potts
 * prior, by iterated conditional modes): biomes in patches, not speckle.
 */
pub fn fit(blocks: &Blocks, m: usize, seed: u64, restarts: usize, smooth: f64) -> Fit {
    let mut best_fit: Option<Fit> = None;
    for r in 0..restarts {
        let mut fit = estimate(blocks, m, kmeans(blocks, m, hash64(seed, r as i64, 1, 0)));
        for _ in 0..40 {
            let mut assign: Vec<u16> = blocks.tokens.iter().map(|t| best(&fit, t, blocks.weight).0).collect();
            if smooth > 0.0 {
                assign = fit.assign.clone();
                for i in 0..assign.len() {
                    let (x, y) = (i % blocks.nx, i / blocks.nx);
                    let mut sides = Vec::with_capacity(4);
                    if x > 0 {
                        sides.push(assign[i - 1]);
                    }
                    if x + 1 < blocks.nx {
                        sides.push(assign[i + 1]);
                    }
                    if y > 0 {
                        sides.push(assign[i - blocks.nx]);
                    }
                    if y + 1 < blocks.ny {
                        sides.push(assign[i + blocks.nx]);
                    }
                    let total = |c: usize| {
                        score(&fit, c, &blocks.tokens[i], blocks.weight)
                            + smooth * sides.iter().filter(|s| **s as usize == c).count() as f64
                    };
                    assign[i] = (0..m).max_by(|a, b| total(*a).total_cmp(&total(*b))).unwrap_or(0) as u16;
                }
            }
            // An emptied component takes the block worst explained.
            for c in 0..m {
                if !assign.contains(&(c as u16)) {
                    let worst = (0..assign.len())
                        .min_by(|a, b| {
                            let s = |i: usize| score(&fit, assign[i] as usize, &blocks.tokens[i], blocks.weight);
                            s(*a).total_cmp(&s(*b))
                        })
                        .unwrap_or(0);
                    assign[worst] = c as u16;
                }
            }
            let changed = assign != fit.assign;
            fit = estimate(blocks, m, assign);
            if !changed {
                break;
            }
        }
        if best_fit.as_ref().is_none_or(|b| fit.bits() < b.bits()) {
            best_fit = Some(fit);
        }
    }
    best_fit.expect("a start")
}

/**
 * Mixtures of 1 to `max_m` components; the one of the shortest description,
 * and each's bits (which leave out the smoothing prior).
 */
pub fn select(blocks: &Blocks, max_m: usize, seed: u64, restarts: usize, smooth: f64) -> (Fit, Vec<Fit>) {
    let fits: Vec<Fit> = (1..=max_m.min(blocks.tokens.len())).map(|m| fit(blocks, m, seed, restarts, smooth)).collect();
    let best = fits.iter().min_by(|a, b| a.bits().total_cmp(&b.bits())).expect("a fit").clone();
    (best, fits)
}
