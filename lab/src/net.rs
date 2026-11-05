//! What the masked models share (masked.rs on cubic grids, masked_fcc.rs
//! on the FCC lattice): the step schedule, k-means for codes, and the
//! network that draws a cell from its slots, trained by Adam.

use worldgen::noise::{Rng, unit};

/** The masked share after step s of `steps` (cosine): 1 at 0, 0 at the last. */
pub fn masked_after(s: usize, steps: usize) -> f64 {
    (std::f64::consts::FRAC_PI_2 * s as f64 / steps as f64).cos()
}

/** The step (1 ..= steps) a cell is drawn at, from a uniform u: the first after which it is no longer masked. */
pub fn step_of(u: f64, steps: usize) -> usize {
    (1..=steps).find(|s| u < 1.0 - masked_after(*s, steps)).unwrap_or(steps)
}

/** Index of the centroid (`dim` floats each) nearest `f`. */
pub fn nearest(centroids: &[f32], dim: usize, f: &[f32]) -> usize {
    let mut best = (0, f32::INFINITY);
    for (c, centroid) in centroids.chunks(dim).enumerate() {
        let d: f32 = centroid.iter().zip(f).map(|(a, b)| (a - b) * (a - b)).sum();
        if d < best.1 {
            best = (c, d);
        }
    }
    best.0
}

/** k-means++ then Lloyd of `points` (`dim` floats each) into at most `k` centroids. */
pub fn kmeans(points: &[f32], dim: usize, k: usize, seed: u64) -> Vec<f32> {
    const ITERATIONS: usize = 16;
    let mut rng = Rng::new(seed);
    let take = points.len() / dim;
    let point = |t: usize| &points[t * dim..][..dim];
    let dist = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
    let k = k.min(take);
    let mut centroids: Vec<f32> = point(rng.below(take as u32) as usize).to_vec();
    let mut near = (0..take).map(|t| dist(point(t), &centroids[..dim])).collect::<Vec<f32>>();
    for _ in 1..k {
        let total: f64 = near.iter().map(|d| *d as f64).sum();
        let mut r = rng.next() * total;
        let mut pick = take - 1;
        for (t, d) in near.iter().enumerate() {
            r -= *d as f64;
            if r <= 0.0 {
                pick = t;
                break;
            }
        }
        let c = point(pick).to_vec();
        near.iter_mut().enumerate().for_each(|(t, d)| *d = d.min(dist(point(t), &c)));
        centroids.extend(c);
    }
    let mut assign = vec![0usize; take];
    for _ in 0..ITERATIONS {
        for (t, a) in assign.iter_mut().enumerate() {
            *a = nearest(&centroids, dim, point(t));
        }
        let mut sum = vec![0f64; k * dim];
        let mut count = vec![0usize; k];
        for t in 0..take {
            count[assign[t]] += 1;
            sum[assign[t] * dim..][..dim].iter_mut().zip(point(t)).for_each(|(s, p)| *s += *p as f64);
        }
        for c in 0..k {
            if count[c] > 0 {
                for d in 0..dim {
                    centroids[c * dim + d] = (sum[c * dim + d] / count[c] as f64) as f32;
                }
            }
        }
    }
    centroids
}

/** The network of one level: embeddings per (slot, state), two ReLU layers, a softmax. */
pub struct Net {
    pub h: usize,
    pub out: usize,
    emb: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
    w3: Vec<f32>,
    b3: Vec<f32>,
}

/** Activations kept for the backward pass. */
#[derive(Default)]
pub struct Pass {
    h1: Vec<f32>,
    h2: Vec<f32>,
    pub p: Vec<f32>,
}

impl Net {
    pub fn parameters(&self) -> usize {
        self.emb.len() + self.w2.len() + self.w3.len() + 2 * self.h + self.out
    }

    pub fn new(rows: usize, h: usize, out: usize, seed: u64) -> Net {
        let mut rng = Rng::new(seed);
        let mut normal = |s: f64| {
            // Box–Muller.
            let (u, v) = (rng.next().max(1e-12), rng.next());
            ((-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos() * s) as f32
        };
        let emb = (0..rows * h).map(|_| normal(0.05)).collect();
        let w2 = (0..h * h).map(|_| normal((2.0 / h as f64).sqrt())).collect();
        let w3 = (0..out * h).map(|_| normal((1.0 / h as f64).sqrt())).collect();
        Net { h, out, emb, b1: vec![0.0; h], w2, b2: vec![0.0; h], w3, b3: vec![0.0; out] }
    }

    pub fn forward(&self, rows: &[usize], pass: &mut Pass) {
        let h = self.h;
        pass.h1.clear();
        pass.h1.extend_from_slice(&self.b1);
        for r in rows {
            pass.h1.iter_mut().zip(&self.emb[r * h..][..h]).for_each(|(a, e)| *a += e);
        }
        pass.h1.iter_mut().for_each(|a| *a = a.max(0.0));
        pass.h2.clear();
        for j in 0..h {
            let s: f32 = self.w2[j * h..][..h].iter().zip(&pass.h1).map(|(w, x)| w * x).sum();
            pass.h2.push((s + self.b2[j]).max(0.0));
        }
        pass.p.clear();
        for o in 0..self.out {
            let s: f32 = self.w3[o * h..][..h].iter().zip(&pass.h2).map(|(w, x)| w * x).sum();
            pass.p.push(s + self.b3[o]);
        }
        let max = pass.p.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut total = 0.0;
        pass.p.iter_mut().for_each(|v| {
            *v = (*v - max).exp();
            total += *v;
        });
        pass.p.iter_mut().for_each(|v| *v /= total);
    }
}

/** Adam's moments for a parameter vector. */
struct Moments {
    m: Vec<f32>,
    v: Vec<f32>,
}

impl Moments {
    fn new(n: usize) -> Moments {
        Moments { m: vec![0.0; n], v: vec![0.0; n] }
    }

    fn step(&mut self, w: &mut [f32], g: &[f32], range: std::ops::Range<usize>, lr: f32, t: i32) {
        const B1: f32 = 0.9;
        const B2: f32 = 0.999;
        let (c1, c2) = (1.0 - B1.powi(t), 1.0 - B2.powi(t));
        for i in range {
            self.m[i] = B1 * self.m[i] + (1.0 - B1) * g[i];
            self.v[i] = B2 * self.v[i] + (1.0 - B2) * g[i] * g[i];
            w[i] -= lr * (self.m[i] / c1) / ((self.v[i] / c2).sqrt() + 1e-8);
        }
    }
}

/**
 * Trains `net` by Adam on cross-entropy, on `total` examples in batches,
 * the learning rate down linearly to a tenth; `example` fills an
 * example's rows and returns its label.
 */
pub fn train(net: &mut Net, total: usize, example: &mut dyn FnMut(&mut Vec<usize>) -> usize) {
    const BATCH: usize = 32;
    const LR: f32 = 2e-3;
    let (h, out) = (net.h, net.out);
    let mut m_emb = Moments::new(net.emb.len());
    let (mut m_b1, mut m_w2, mut m_b2, mut m_w3, mut m_b3) =
        (Moments::new(h), Moments::new(h * h), Moments::new(h), Moments::new(out * h), Moments::new(out));
    let (mut g_emb, mut g_b1, mut g_w2, mut g_b2, mut g_w3, mut g_b3) =
        (vec![0f32; net.emb.len()], vec![0f32; h], vec![0f32; h * h], vec![0f32; h], vec![0f32; out * h], vec![0f32; out]);
    let mut touched: Vec<usize> = Vec::new();
    let mut pass = Pass::default();
    let mut rows = Vec::new();
    let (mut d1, mut d2) = (vec![0f32; h], vec![0f32; h]);
    let batches = total / BATCH;
    let mut t = 0;
    for b in 0..batches {
        // Learning rate down linearly to a tenth.
        let lr = LR * (1.0 - 0.9 * b as f32 / batches as f32);
        for _ in 0..BATCH {
            let target = example(&mut rows);
            net.forward(&rows, &mut pass);
            // Output gradient: p − onehot, over the batch.
            let scale = 1.0 / BATCH as f32;
            d2.iter_mut().for_each(|d| *d = 0.0);
            for o in 0..out {
                let g = (pass.p[o] - if o == target { 1.0 } else { 0.0 }) * scale;
                g_b3[o] += g;
                for j in 0..h {
                    g_w3[o * h + j] += g * pass.h2[j];
                    d2[j] += g * net.w3[o * h + j];
                }
            }
            d2.iter_mut().zip(&pass.h2).for_each(|(d, a)| {
                if *a <= 0.0 {
                    *d = 0.0
                }
            });
            d1.iter_mut().for_each(|d| *d = 0.0);
            for j in 0..h {
                if d2[j] == 0.0 {
                    continue;
                }
                g_b2[j] += d2[j];
                for i in 0..h {
                    g_w2[j * h + i] += d2[j] * pass.h1[i];
                    d1[i] += d2[j] * net.w2[j * h + i];
                }
            }
            d1.iter_mut().zip(&pass.h1).for_each(|(d, a)| {
                if *a <= 0.0 {
                    *d = 0.0
                }
            });
            g_b1.iter_mut().zip(&d1).for_each(|(g, d)| *g += d);
            for r in &rows {
                g_emb[r * h..][..h].iter_mut().zip(&d1).for_each(|(g, d)| *g += d);
                touched.push(*r);
            }
        }
        t += 1;
        touched.sort_unstable();
        touched.dedup();
        for r in &touched {
            m_emb.step(&mut net.emb, &g_emb, r * h..(r + 1) * h, lr, t);
            g_emb[r * h..][..h].iter_mut().for_each(|g| *g = 0.0);
        }
        touched.clear();
        m_b1.step(&mut net.b1, &g_b1, 0..h, lr, t);
        m_w2.step(&mut net.w2, &g_w2, 0..h * h, lr, t);
        m_b2.step(&mut net.b2, &g_b2, 0..h, lr, t);
        m_w3.step(&mut net.w3, &g_w3, 0..out * h, lr, t);
        m_b3.step(&mut net.b3, &g_b3, 0..out, lr, t);
        for g in [&mut g_b1, &mut g_w2, &mut g_b2, &mut g_w3, &mut g_b3] {
            g.iter_mut().for_each(|v| *v = 0.0);
        }
    }
}

/** Mean cross-entropy, bits, of CELLS examples. */
pub fn bits(net: &Net, example: &mut dyn FnMut(&mut Vec<usize>) -> usize) -> f64 {
    const CELLS: usize = 20_000;
    let mut rows = Vec::new();
    let mut pass = Pass::default();
    let mut bits = 0.0;
    for _ in 0..CELLS {
        let target = example(&mut rows);
        net.forward(&rows, &mut pass);
        bits -= (pass.p[target].max(1e-9) as f64).log2();
    }
    bits / CELLS as f64
}

/** A label drawn from `p` by the hash `h`. */
pub fn draw(p: &[f32], h: u64) -> u16 {
    let w: Vec<f64> = p.iter().map(|v| *v as f64).collect();
    let total: f64 = w.iter().sum();
    let mut r = unit((h >> 32) as u32) * total;
    for (i, v) in w.iter().enumerate() {
        r -= v;
        if r < 0.0 {
            return i as u16;
        }
    }
    (p.len() - 1) as u16
}
