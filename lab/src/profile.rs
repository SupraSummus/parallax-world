//! A profile: how a set of maps is laid out, label by label, as
//! distributions, set against real maps' to say what differs. Nothing in it
//! knows what a label is: a street falling into pieces shows as the
//! street's parts getting smaller, houses away from streets as roofs
//! getting farther from streets; a house in a field is no fault, too many
//! of them are.
//!
//! The critic's network (critic.rs) sees what its windows hold, not how far
//! a part runs on past them, so a profile is what sees parts joined.

use worldgen::noise::Rng;

use crate::geometry::components;
use crate::image::Image;

/** Bins of a size or distance, in doublings: 1, 2–3, 4–7, … up to the last, open. */
const BINS: usize = 16;

fn bin(s: usize) -> usize {
    (usize::BITS - s.max(1).leading_zeros() - 1).min(BINS as u32 - 1) as usize
}

/** Distances (largest of the two axes) from each cell to the nearest of label `l`; u32::MAX if the map holds none. */
fn distances(map: &Image, l: u16) -> Vec<u32> {
    let (w, h) = (map.w, map.h);
    let mut d: Vec<u32> = map.data.iter().map(|v| if *v == l { 0 } else { u32::MAX }).collect();
    let relax = |d: &mut Vec<u32>, x: usize, y: usize, dx: i64, dy: i64| {
        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
            let n = d[ny as usize * w + nx as usize].saturating_add(1);
            let i = y * w + x;
            d[i] = d[i].min(n);
        }
    };
    for y in 0..h {
        for x in 0..w {
            for (dx, dy) in [(-1, 0), (-1, -1), (0, -1), (1, -1)] {
                relax(&mut d, x, y, dx, dy);
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            for (dx, dy) in [(1, 0), (1, 1), (0, 1), (-1, 1)] {
                relax(&mut d, x, y, dx, dy);
            }
        }
    }
    d
}

/** Cell counts by bin, summed over maps: per label, and per pair of labels for `near`. */
pub struct Profile {
    n: usize,
    /** Per label: its cells by the size of the 4-connected part they are in. */
    sizes: Vec<[f64; BINS]>,
    /** Per label: its cells by how far the nearest cell of another label is (half its width, where it is thin). */
    widths: Vec<[f64; BINS]>,
    /** Per pair a, b: a's cells by how far the nearest of b is, in maps holding b (whether a map does is its plan's to say). */
    near: Vec<[f64; BINS]>,
}

/** How one distribution of a sample's profile differs from the real one's. */
pub struct Difference {
    pub family: &'static str,
    pub label: usize,
    pub other: Option<usize>,
    /** Earth mover's distance in bins: how many doublings, on average, the sample's cells sit off the real ones'. */
    pub moved: f64,
    /** The label's share of the real maps: what the family's mean weighs it by. */
    pub weight: f64,
}

pub const FAMILIES: [&str; 3] = ["sizes", "widths", "near"];

impl Profile {
    pub fn of(maps: &[Image], n: usize) -> Profile {
        let mut p = Profile { n, sizes: vec![[0.0; BINS]; n], widths: vec![[0.0; BINS]; n], near: vec![[0.0; BINS]; n * n] };
        for map in maps {
            for l in 0..n {
                for part in components(map, l as u16) {
                    p.sizes[l][bin(part.len())] += part.len() as f64;
                }
            }
            let d: Vec<Vec<u32>> = (0..n).map(|l| distances(map, l as u16)).collect();
            for (i, a) in map.data.iter().enumerate() {
                let a = *a as usize;
                let mut width = u32::MAX;
                for (b, d) in d.iter().enumerate().filter(|(b, d)| *b != a && d[i] != u32::MAX) {
                    p.near[a * n + b][bin(d[i] as usize)] += 1.0;
                    width = width.min(d[i]);
                }
                p.widths[a][bin(width as usize)] += 1.0;
            }
        }
        p
    }

    /** Every distribution of `self` against `real`'s, where the real maps hold the label. */
    pub fn differences(&self, real: &Profile) -> Vec<Difference> {
        let all: f64 = real.sizes.iter().flatten().sum();
        let share = |l: usize| real.sizes[l].iter().sum::<f64>() / all;
        let mut out = Vec::new();
        for l in (0..self.n).filter(|l| share(*l) > 0.0) {
            out.push(Difference {
                family: FAMILIES[0],
                label: l,
                other: None,
                moved: moved(&self.sizes[l], &real.sizes[l]),
                weight: share(l),
            });
            out.push(Difference {
                family: FAMILIES[1],
                label: l,
                other: None,
                moved: moved(&self.widths[l], &real.widths[l]),
                weight: share(l),
            });
            for b in (0..self.n).filter(|b| *b != l && share(*b) > 0.0) {
                let (s, r) = (&self.near[l * self.n + b], &real.near[l * self.n + b]);
                out.push(Difference { family: FAMILIES[2], label: l, other: Some(b), moved: moved(s, r), weight: share(l) });
            }
        }
        out
    }
}

/**
 * Each of `diffs` less what real places' differ by (`baseline`, real maps
 * other than `real`'s against them, in the same order), at least 0: how
 * far beyond the spread of real places a sample is. Real places differ a
 * lot in where they have water, little in how wide their streets are.
 */
pub fn beyond(diffs: &mut [Difference], baseline: &[Difference]) {
    assert_eq!(diffs.len(), baseline.len(), "differences from the same real profile");
    for (d, b) in diffs.iter_mut().zip(baseline) {
        d.moved = (d.moved - b.moved).max(0.0);
    }
}

/** Per family, the differences' mean weighed by their labels' shares. */
pub fn by_family(diffs: &[Difference]) -> [f64; 3] {
    FAMILIES.map(|f| {
        let (sum, weight) = diffs.iter().filter(|d| d.family == f).fold((0.0, 0.0), |(s, w), d| (s + d.moved * d.weight, w + d.weight));
        if weight > 0.0 { sum / weight } else { 0.0 }
    })
}

/**
 * Earth mover's distance between two histograms over the same ordered
 * bins, each normalised; none if either is empty: whether maps hold a
 * label at all is their plan's to say.
 */
fn moved(a: &[f64; BINS], b: &[f64; BINS]) -> f64 {
    let (sa, sb) = (a.iter().sum::<f64>(), b.iter().sum::<f64>());
    if sa == 0.0 || sb == 0.0 {
        return 0.0;
    }
    let (mut ca, mut cb, mut out) = (0.0, 0.0, 0.0);
    for i in 0..BINS - 1 {
        ca += a[i] / sa;
        cb += b[i] / sb;
        out += (ca - cb).abs();
    }
    out
}

/**
 * `map` with label `l` cut: a `gap` × `gap` square of it turned to `fill`
 * in each `every` × `every` tile that holds it. Damage whose effect is
 * known (parts fall apart, a little of the label goes), to see that a
 * measure sees it.
 */
pub fn cut(map: &Image, l: u16, fill: u16, every: usize, gap: usize, seed: u64) -> Image {
    let mut out = map.clone();
    let mut rng = Rng::new(seed);
    for ty in (0..map.h).step_by(every) {
        for tx in (0..map.w).step_by(every) {
            // A cell of the label in the tile, if a few tries find one.
            let found = (0..20)
                .map(|_| (tx + rng.below(every as u32) as usize, ty + rng.below(every as u32) as usize))
                .find(|(x, y)| x + gap <= map.w && y + gap <= map.h && map.at(*x, *y) == l);
            let Some((x, y)) = found else { continue };
            for j in 0..gap {
                for i in 0..gap {
                    if out.at(x + i, y + j) == l {
                        out.set(x + i, y + j, fill);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_lines_fall_into_parts_and_join_what_they_parted() {
        // Streets (1) a metre wide every 8 m across fields (0).
        let map = Image::new(128, 128, (0..128 * 128).map(|i| ((i / 128) % 8 == 0) as u16).collect());
        let real = Profile::of(std::slice::from_ref(&map), 2);
        assert!(by_family(&real.differences(&real)).iter().all(|d| *d == 0.0));
        let cut = Profile::of(&[cut(&map, 1, 0, 16, 2, 1)], 2);
        let diffs = cut.differences(&real);
        let of = |f: &str, l: usize| diffs.iter().find(|d| d.family == f && d.label == l && d.other.is_none()).unwrap().moved;
        assert!(of("sizes", 1) > 2.0, "cut streets are in parts a few doublings smaller: {}", of("sizes", 1));
        assert!(of("sizes", 0) > 2.0, "the fields between them join: {}", of("sizes", 0));
        assert!(of("widths", 0) < 0.1 && of("widths", 1) < 0.1, "widths stay");
    }
}
