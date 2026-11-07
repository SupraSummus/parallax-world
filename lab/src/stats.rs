//! A map's statistics by scale: label shares and co-occurrence at a
//! distance; and how far two distributions are apart.

use crate::image::Image;

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
