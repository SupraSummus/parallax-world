//! A plan the guided models draw a map to (exemplar.rs, masked.rs): its
//! context, what kind of place each part of it is, as the shares of
//! groups of labels around (built, forest, fields, water), which the
//! critic matches real windows by too (critic.rs); and its lines, the
//! networks given exactly (main roads, rivers), which shares can't hold:
//! the models draw everything else around them.

use crate::image::Image;

pub struct Plan {
    pub context: Context,
    pub lines: Option<Lines>,
}

/** Per cell of a grid `cell` map cells wide, the shares of `groups` in the cells `blur` around it. */
#[derive(Clone, Debug)]
pub struct Context {
    pub cell: usize,
    pub w: usize,
    pub h: usize,
    pub dim: usize,
    pub v: Vec<f32>,
}

impl Context {
    pub fn of(map: &Image, groups: &[&[u16]], cell: usize, blur: usize) -> Context {
        let (w, h, dim) = (map.w.div_ceil(cell), map.h.div_ceil(cell), groups.len());
        let mut raw = vec![0f32; w * h * dim];
        let mut count = vec![0f32; w * h];
        for y in 0..map.h {
            for x in 0..map.w {
                let c = (y / cell) * w + x / cell;
                count[c] += 1.0;
                let l = map.at(x, y);
                if let Some(g) = groups.iter().position(|g| g.contains(&l)) {
                    raw[c * dim + g] += 1.0;
                }
            }
        }
        let mut v = vec![0f32; w * h * dim];
        let b = blur as i64;
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let (mut sum, mut cells) = (vec![0f32; dim], 0f32);
                for j in (y - b).max(0)..=(y + b).min(h as i64 - 1) {
                    for i in (x - b).max(0)..=(x + b).min(w as i64 - 1) {
                        let c = (j * w as i64 + i) as usize;
                        sum.iter_mut().zip(&raw[c * dim..][..dim]).for_each(|(s, r)| *s += r);
                        cells += count[c];
                    }
                }
                let c = (y * w as i64 + x) as usize;
                v[c * dim..][..dim].iter_mut().zip(&sum).for_each(|(o, s)| *o = s / cells.max(1.0));
            }
        }
        Context { cell, w, h, dim, v }
    }

    /** The context at cell (x, y) of the map, clamped to it. */
    pub fn at(&self, x: i32, y: i32) -> &[f32] {
        let cx = (x.max(0) as usize / self.cell).min(self.w - 1);
        let cy = (y.max(0) as usize / self.cell).min(self.h - 1);
        &self.v[(cy * self.w + cx) * self.dim..][..self.dim]
    }

    /** Mean absolute difference from another context of the same grid, per group. */
    pub fn error(&self, other: &Context) -> f64 {
        self.v.iter().zip(&other.v).map(|(a, b)| (a - b).abs() as f64).sum::<f64>() / self.v.len() as f64
    }
}

/** Where a map's lines have no label. */
pub const NONE: u16 = u16::MAX;

/** Per cell of a map, the label of the line running there (one of `labels`), or NONE; and a summed-area table per label. */
#[derive(Clone, Debug)]
pub struct Lines {
    pub labels: Vec<u16>,
    map: Image,
    sat: Vec<Vec<u32>>,
}

impl Lines {
    pub fn new(map: Image, labels: &[u16]) -> Lines {
        let (w, h) = (map.w, map.h);
        let sat = labels
            .iter()
            .map(|l| {
                let mut t = vec![0u32; (w + 1) * (h + 1)];
                for y in 0..h {
                    for x in 0..w {
                        let at = |x: usize, y: usize| y * (w + 1) + x;
                        t[at(x + 1, y + 1)] = (map.at(x, y) == *l) as u32 + t[at(x, y + 1)] + t[at(x + 1, y)] - t[at(x, y)];
                    }
                }
                t
            })
            .collect();
        Lines { labels: labels.to_vec(), map, sat }
    }

    /** A map's own cells of `labels`, as lines. */
    pub fn of(map: &Image, labels: &[u16]) -> Lines {
        Lines::new(Image::new(map.w, map.h, map.data.iter().map(|l| if labels.contains(l) { *l } else { NONE }).collect()), labels)
    }

    /** The line at (x, y), if any (none beyond the map). */
    pub fn at(&self, x: i64, y: i64) -> Option<u16> {
        let inside = x >= 0 && y >= 0 && (x as usize) < self.map.w && (y as usize) < self.map.h;
        Some(if inside { self.map.at(x as usize, y as usize) } else { NONE }).filter(|l| *l != NONE)
    }

    /** Each label's share of the square `side` wide from (x, y), what is beyond the map counting as no line. */
    pub fn shares(&self, x: i64, y: i64, side: i64, out: &mut [f32]) {
        let (w, h) = (self.map.w as i64, self.map.h as i64);
        let (x0, y0, x1, y1) = (x.clamp(0, w), y.clamp(0, h), (x + side).clamp(0, w), (y + side).clamp(0, h));
        for (o, t) in out.iter_mut().zip(&self.sat) {
            let at = |x: i64, y: i64| t[(y * (w + 1) + x) as usize];
            *o = (at(x1, y1) + at(x0, y0) - at(x0, y1) - at(x1, y0)) as f32 / (side * side) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_is_the_shares_of_its_groups_around() {
        // Left half label 1, right half 0: cells of 4, blurred one cell each way.
        let map = Image::new(16, 4, (0..64).map(|i| ((i % 16) < 8) as u16).collect());
        let c = Context::of(&map, &[&[1]], 4, 1);
        let shares: Vec<f32> = (0..4).map(|x| c.at(x * 4, 0)[0]).collect();
        assert_eq!(shares, vec![1.0, 2.0 / 3.0, 1.0 / 3.0, 0.0]);
    }
}
