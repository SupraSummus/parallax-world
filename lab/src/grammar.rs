//! A split grammar learned from examples: rectangles, not pixels.
//!
//! Each training crop is parsed into a binary tree of rectangles: a
//! rectangle is cut, along x or y, where the cut most lowers the entropy of
//! the labels on either side, until no cut gains enough (a leaf). The
//! rectangles of all the trees are clustered by their labels' shares into
//! kinds. That is the grammar: a kind's rules are its rectangles, each
//! either a leaf (its pixels) or a cut (an axis, a place, two kinds).
//!
//! A new crop is drawn top down: a rectangle of a kind takes the rule of a
//! rectangle of that kind of about its size (chosen by its seed), cut at the
//! same distance from the nearer end, or filled with the leaf's pixels,
//! scaled. A rectangle's draw depends on its seed and its own extent only,
//! and a box only descends into the rectangles it meets: a box is the same
//! alone or as part of the whole. A cut runs the whole rectangle, so what
//! it separates (a street) is straight and goes on as far.

use std::collections::HashMap;

use worldgen::noise::{Rng, hash64};

use crate::image::Image;

/** A rectangle of a parse: its extent in its crop, labels' counts, cut (axis 0 x, 1 y; at), children, crop, kind. */
#[derive(Clone, Debug)]
pub struct Node {
    pub x0: usize,
    pub y0: usize,
    pub w: usize,
    pub h: usize,
    pub hist: Vec<u32>,
    pub cut: Option<(usize, usize)>,
    pub kids: [usize; 2],
    pub crop: usize,
    pub kind: usize,
}

/**
 * A cut must gain this many bits in all, and per cell of the rectangle;
 * a rectangle longer than MAX_LEAF is cut anyway, so that no leaf is a
 * whole town copied.
 */
const MIN_GAIN: f64 = 16.0;
const MIN_GAIN_PER_CELL: f64 = 0.02;
pub const MAX_LEAF: usize = 16;

fn entropy(hist: &[u32], total: f64) -> f64 {
    hist.iter().filter(|c| **c > 0).map(|c| *c as f64 / total).map(|p| -p * p.log2()).sum::<f64>() * total
}

/** Parses the rectangle (x0, y0) w × h of `image` into `nodes`, returning its index. */
fn parse(image: &Image, n: usize, crop: usize, (x0, y0, w, h): (usize, usize, usize, usize), nodes: &mut Vec<Node>) -> usize {
    let mut cols = vec![vec![0u32; n]; w];
    let mut rows = vec![vec![0u32; n]; h];
    let mut hist = vec![0u32; n];
    for (y, row) in rows.iter_mut().enumerate() {
        for (x, col) in cols.iter_mut().enumerate() {
            let l = image.at(x0 + x, y0 + y) as usize;
            col[l] += 1;
            row[l] += 1;
            hist[l] += 1;
        }
    }
    let area = (w * h) as f64;
    let whole = entropy(&hist, area);
    let mut best: Option<(f64, usize, usize)> = None;
    for (axis, lines) in [(0, &cols), (1, &rows)] {
        let mut left = vec![0u32; n];
        let across = if axis == 0 { h } else { w } as f64;
        for at in 1..lines.len() {
            left.iter_mut().zip(&lines[at - 1]).for_each(|(a, b)| *a += b);
            let right: Vec<u32> = hist.iter().zip(&left).map(|(t, l)| t - l).collect();
            let (al, ar) = (at as f64 * across, area - at as f64 * across);
            let gain = whole - entropy(&left, al) - entropy(&right, ar);
            if best.is_none_or(|b| gain > b.0 + 1e-9) {
                best = Some((gain, axis, at));
            }
        }
    }
    let at = nodes.len();
    nodes.push(Node { x0, y0, w, h, hist, cut: None, kids: [0, 0], crop, kind: 0 });
    let gains = best.is_some_and(|(gain, _, _)| gain >= MIN_GAIN && gain / area >= MIN_GAIN_PER_CELL);
    // Cut anyway where too long: where it gains most, or in the middle of the longer side if no cut gains.
    let best = if gains {
        best
    } else if w.max(h) <= MAX_LEAF {
        None
    } else if best.is_some_and(|b| b.0 >= 1.0) {
        best
    } else {
        Some(if w >= h { (0.0, 0, w / 2) } else { (0.0, 1, h / 2) })
    };
    if let Some((_, axis, cut)) = best {
        let (a, b) =
            if axis == 0 { ((x0, y0, cut, h), (x0 + cut, y0, w - cut, h)) } else { ((x0, y0, w, cut), (x0, y0 + cut, w, h - cut)) };
        let ka = parse(image, n, crop, a, nodes);
        let kb = parse(image, n, crop, b, nodes);
        nodes[at].cut = Some((axis, cut));
        nodes[at].kids = [ka, kb];
    }
    at
}

pub struct Grammar {
    pub n: usize,
    pub kinds: usize,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub crops: Vec<Image>,
    /**
     * Nodes by context (0 any, else 1 + their parent's kind × 2 + which
     * child), kind and size (half-octaves of w and h).
     */
    by_size: HashMap<(usize, usize, i32, i32), Vec<usize>>,
    /** Whether a rule is chosen by the parent's kind and side too, when one of about the size has been seen there. */
    pub contextual: bool,
}

fn octaves(s: usize) -> i32 {
    ((s as f64).log2() * 2.0).round() as i32
}

/** How leaves are filled: with their rule's pixels, or its commonest label (so that all the shapes are the cuts'). */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leaves {
    Pixels,
    Commonest,
}

/** What a draw took: rectangles drawn, and which training crop each cell's leaf came from. */
pub struct Drawn {
    pub image: Image,
    pub source: Vec<u32>,
    pub rectangles: usize,
}

impl Grammar {
    /** The grammar of `crops` (labels below `n`), its rectangles in `kinds` kinds. */
    pub fn learn(crops: &[Image], n: usize, kinds: usize, seed: u64, contextual: bool) -> Grammar {
        let mut nodes = Vec::new();
        let roots: Vec<usize> = crops.iter().enumerate().map(|(i, c)| parse(c, n, i, (0, 0, c.w, c.h), &mut nodes)).collect();
        let shares: Vec<Vec<f64>> = nodes
            .iter()
            .map(|nd| {
                let total = (nd.w * nd.h) as f64;
                nd.hist.iter().map(|c| *c as f64 / total).collect()
            })
            .collect();
        let kind = kmeans(&shares, kinds, seed);
        nodes.iter_mut().zip(&kind).for_each(|(nd, k)| nd.kind = *k);
        let mut context = vec![0; nodes.len()];
        for nd in &nodes {
            if nd.cut.is_some() {
                context[nd.kids[0]] = 1 + nd.kind * 2;
                context[nd.kids[1]] = 2 + nd.kind * 2;
            }
        }
        let mut by_size: HashMap<(usize, usize, i32, i32), Vec<usize>> = HashMap::new();
        for (i, nd) in nodes.iter().enumerate() {
            by_size.entry((0, nd.kind, octaves(nd.w), octaves(nd.h))).or_default().push(i);
            if context[i] > 0 {
                by_size.entry((context[i], nd.kind, octaves(nd.w), octaves(nd.h))).or_default().push(i);
            }
        }
        Grammar { n, kinds, nodes, roots, crops: crops.to_vec(), by_size, contextual }
    }

    /**
     * A rule for a rectangle of `kind`, w × h, in `context`: a node of that
     * kind of the nearest size (in that context, if one is within a
     * half-octave or two), chosen by `h`.
     */
    fn rule(&self, context: usize, kind: usize, w: usize, hh: usize, h: u64) -> Option<usize> {
        let (ow, oh) = (octaves(w), octaves(hh));
        let tries = if self.contextual && context > 0 { vec![(context, 2i32), (0, 12)] } else { vec![(0, 12)] };
        for (c, radius) in tries {
            for r in 0..=radius {
                let mut found: Vec<usize> = Vec::new();
                for dw in -r..=r {
                    for dh in -r..=r {
                        if dw.abs().max(dh.abs()) == r
                            && let Some(v) = self.by_size.get(&(c, kind, ow + dw, oh + dh))
                        {
                            found.extend(v);
                        }
                    }
                }
                if !found.is_empty() {
                    found.sort_unstable();
                    return Some(found[(h % found.len() as u64) as usize]);
                }
            }
        }
        None
    }

    /** The crop of `seed`, w × h like the training crops, over the box (x0, y0) bw × bh of it, alone. */
    pub fn sample(&self, seed: u64, (x0, y0, bw, bh): (usize, usize, usize, usize), leaves: Leaves) -> Drawn {
        let mut drawn = Drawn { image: Image::filled(bw, bh, 0), source: vec![0; bw * bh], rectangles: 0 };
        let root = self.roots[(hash64(seed, 0, 0, 1) % self.roots.len() as u64) as usize];
        let (w, h) = (self.crops[0].w, self.crops[0].h);
        self.draw(hash64(seed, 0, 0, 2), Some(root), 0, self.nodes[root].kind, (0, 0, w, h), (x0, y0, bw, bh), leaves, &mut drawn);
        drawn
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        h: u64,
        rule: Option<usize>,
        context: usize,
        kind: usize,
        r: (usize, usize, usize, usize),
        bx: (usize, usize, usize, usize),
        leaves: Leaves,
        out: &mut Drawn,
    ) {
        let (rx, ry, rw, rh) = r;
        let (bx0, by0, bw, bh) = bx;
        if rx >= bx0 + bw || ry >= by0 + bh || rx + rw <= bx0 || ry + rh <= by0 {
            return;
        }
        out.rectangles += 1;
        let Some(e) = rule.or_else(|| self.rule(context, kind, rw, rh, h)) else { return };
        let node = &self.nodes[e];
        if let Some((axis, cut)) = node.cut {
            let (le, lt) = if axis == 0 { (node.w, rw) } else { (node.h, rh) };
            if lt >= 2 {
                let at = if cut * 2 <= le { cut } else { lt.saturating_sub(le - cut) }.clamp(1, lt - 1);
                let (a, b) =
                    if axis == 0 { ((rx, ry, at, rh), (rx + at, ry, rw - at, rh)) } else { ((rx, ry, rw, at), (rx, ry + at, rw, rh - at)) };
                for (side, (kid, part)) in node.kids.iter().zip([a, b]).enumerate() {
                    self.draw(hash64(h, side as i64, 0, 3), None, 1 + kind * 2 + side, self.nodes[*kid].kind, part, bx, leaves, out);
                }
                return;
            }
        }
        // A leaf: the rule's pixels, scaled to the rectangle, or its commonest label.
        let crop = &self.crops[node.crop];
        let common = (0..self.n).max_by_key(|l| (node.hist[*l], std::cmp::Reverse(*l))).unwrap_or(0) as u16;
        for y in ry.max(by0)..(ry + rh).min(by0 + bh) {
            for x in rx.max(bx0)..(rx + rw).min(bx0 + bw) {
                let (u, v) = (node.x0 + (x - rx) * node.w / rw, node.y0 + (y - ry) * node.h / rh);
                out.image.set(x - bx0, y - by0, if leaves == Leaves::Pixels { crop.at(u, v) } else { common });
                out.source[(y - by0) * bw + x - bx0] = node.crop as u32;
            }
        }
    }
}

/** Lloyd's k-means of `points` into `k`, from k-means++, deterministic. */
fn kmeans(points: &[Vec<f64>], k: usize, seed: u64) -> Vec<usize> {
    let d2 = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>();
    let mut rng = Rng::new(seed);
    let mut centres = vec![points[(rng.next_u64() % points.len() as u64) as usize].clone()];
    let mut nearest: Vec<f64> = points.iter().map(|p| d2(p, &centres[0])).collect();
    while centres.len() < k {
        let total: f64 = nearest.iter().sum();
        if total <= 0.0 {
            break;
        }
        let mut u = rng.next() * total;
        let pick = nearest.iter().position(|d| {
            u -= d;
            u <= 0.0
        });
        let c = points[pick.unwrap_or(points.len() - 1)].clone();
        nearest.iter_mut().zip(points).for_each(|(n, p)| *n = n.min(d2(p, &c)));
        centres.push(c);
    }
    let mut assign = vec![0; points.len()];
    for _ in 0..30 {
        let next: Vec<usize> = points
            .iter()
            .map(|p| (0..centres.len()).min_by(|a, b| d2(p, &centres[*a]).total_cmp(&d2(p, &centres[*b]))).unwrap_or(0))
            .collect();
        let dim = points[0].len();
        let mut sum = vec![vec![0.0; dim]; centres.len()];
        let mut count = vec![0usize; centres.len()];
        for (p, a) in points.iter().zip(&next) {
            sum[*a].iter_mut().zip(p).for_each(|(s, x)| *s += x);
            count[*a] += 1;
        }
        for (c, (s, n)) in centres.iter_mut().zip(sum.iter().zip(&count)) {
            if *n > 0 {
                *c = s.iter().map(|x| x / *n as f64).collect();
            }
        }
        if next == assign {
            break;
        }
        assign = next;
    }
    assign
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Crops of 64 with a street (1) across at a place of their own, roofs (2) in rectangles beside it. */
    fn crops() -> Vec<Image> {
        (0..8)
            .map(|s| {
                let street = 20 + 3 * s;
                Image::new(
                    64,
                    64,
                    (0..64 * 64)
                        .map(|i| {
                            let (x, y) = (i % 64, i / 64);
                            if (street..street + 5).contains(&y) {
                                1
                            } else if (street + 6..street + 14).contains(&y) && x % 10 < 7 {
                                2
                            } else {
                                0
                            }
                        })
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_parse_cuts_along_the_street() {
        let g = Grammar::learn(&crops(), 3, 4, 1, true);
        let root = &g.nodes[g.roots[0]];
        assert_eq!(root.cut.map(|c| c.0), Some(1), "first cut along y");
    }

    #[test]
    fn a_box_alone_is_the_same_box_of_the_whole() {
        let g = Grammar::learn(&crops(), 3, 4, 1, true);
        for seed in 0..4 {
            let whole = g.sample(seed, (0, 0, 64, 64), Leaves::Pixels).image;
            for (x, y, w, h) in [(0, 0, 13, 9), (37, 51, 20, 7), (63, 63, 1, 1), (5, 3, 40, 40)] {
                assert_eq!(g.sample(seed, (x, y, w, h), Leaves::Pixels).image, whole.crop(x, y, w, h), "box {x} {y} {w} {h}");
            }
        }
    }
}
