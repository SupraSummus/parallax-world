//! Scores: a segmentation against the truth (confusion, adjusted Rand
//! index), and a map's statistics by scale (label shares,
//! co-occurrence at a distance, sizes of patches).

use crate::image::Image;

/** Counts of (predicted, true) labels, `p` × `t`. */
pub fn confusion(predicted: &[u16], truth: &[u16], p: usize, t: usize) -> Vec<Vec<u64>> {
    let mut c = vec![vec![0u64; t]; p];
    for (a, b) in predicted.iter().zip(truth) {
        c[*a as usize][*b as usize] += 1;
    }
    c
}

/** Adjusted Rand index of a confusion matrix: 1 for the same partition, about 0 for a random one. */
pub fn ari(c: &[Vec<u64>]) -> f64 {
    let pairs = |n: u64| (n * n.saturating_sub(1)) as f64 / 2.0;
    let n: u64 = c.iter().flatten().sum();
    let index: f64 = c.iter().flatten().map(|x| pairs(*x)).sum();
    let rows: f64 = c.iter().map(|r| pairs(r.iter().sum())).sum();
    let t = c.first().map_or(0, |r| r.len());
    let cols: f64 = (0..t).map(|j| pairs(c.iter().map(|r| r[j]).sum())).sum();
    let expected = rows * cols / pairs(n).max(1.0);
    let max = (rows + cols) / 2.0;
    if (max - expected).abs() < 1e-12 { 1.0 } else { (index - expected) / (max - expected) }
}

/** Each label's share of the cells. */
pub fn shares(image: &Image, n: usize) -> Vec<f64> {
    let mut s = vec![0.0; n];
    for l in &image.data {
        s[*l as usize] += 1.0 / image.data.len() as f64;
    }
    s
}

/** The joint distribution of the labels of cells `d` apart, along x and along y alike, n × n. */
pub fn cooccurrence(image: &Image, n: usize, d: usize) -> Vec<f64> {
    let mut joint = vec![0.0; n * n];
    let mut total = 0.0f64;
    for y in 0..image.h {
        for x in 0..image.w {
            let a = image.at(x, y) as usize;
            if x + d < image.w {
                joint[a * n + image.at(x + d, y) as usize] += 1.0;
                total += 1.0;
            }
            if y + d < image.h {
                joint[a * n + image.at(x, y + d) as usize] += 1.0;
                total += 1.0;
            }
        }
    }
    joint.iter_mut().for_each(|j| *j /= total.max(1.0));
    joint
}

/** Total variation distance between two distributions. */
pub fn tv(a: &[f64], b: &[f64]) -> f64 {
    0.5 * a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>()
}

/** The chance that cells `d` apart have the same label: how coherent the map is at that distance. */
pub fn same(joint: &[f64], n: usize) -> f64 {
    (0..n).map(|a| joint[a * n + a]).sum()
}

/**
 * Patches, the 4-connected parts of one label, of the labels `wanted`:
 * their number, and the mean size of the patch a cell of them is in (area-weighted, cells).
 */
pub fn patches(image: &Image, wanted: &dyn Fn(u16) -> bool) -> (usize, f64) {
    let mut seen = vec![false; image.data.len()];
    let (mut count, mut cells, mut squares) = (0usize, 0.0, 0.0);
    let mut stack = Vec::new();
    for start in 0..image.data.len() {
        let label = image.data[start];
        if seen[start] || !wanted(label) {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut size = 0.0;
        while let Some(i) = stack.pop() {
            size += 1.0;
            let (x, y) = (i % image.w, i / image.w);
            let mut visit = |j: usize| {
                if !seen[j] && image.data[j] == label {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < image.w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - image.w);
            }
            if y + 1 < image.h {
                visit(i + image.w);
            }
        }
        count += 1;
        cells += size;
        squares += size * size;
    }
    (count, if cells > 0.0 { squares / cells } else { 0.0 })
}
