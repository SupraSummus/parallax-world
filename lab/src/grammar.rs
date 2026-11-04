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
//!
//! Shared splits: what goes on across a cut. Where a strip of one label
//! (a cross street) runs across a cut in training, on both sides of it,
//! the cut's rectangle keeps the strip as a guide it gives both sides, and
//! a cut on a guide's edge below it is marked as snapping to it; each
//! rectangle keeps the guides that crossed it. Drawn, a rule passes its
//! guides down to all its descendants, which read them as part of their
//! context (still local: they are their ancestors' draws): a rectangle
//! takes, of the rules of its kind and about its size, one whose guides in
//! training fit those crossing it best, and a snapping cut moves to the
//! nearest guide's edge. Guides a rectangle inherits win over its rule's
//! own, so a grid set high holds throughout.
//!
//! Regimes: rules can be marked as texture, where the grammar stops and
//! another model (regimes.rs) fills the rectangle, whatever its size.

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
    /** Strips both sides of the cut hold, along the other axis: (lo, hi) from the rectangle's start on that axis. */
    pub shared: Vec<(usize, usize)>,
    /** Whether the cut lies on a guide of a rectangle above: on its low edge or its high one. */
    pub snap: Option<Edge>,
    /** The guides from above crossing it, per axis: their centres from its start on that axis. */
    pub crossing: [Vec<usize>; 2],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Lo,
    Hi,
}

/**
 * A shared strip of a cut: lines across the cut (2 to STRIP_WIDE of them
 * side by side) each STRIP_SHARE one label for REACH on both sides of it,
 * the same label throughout, and not the rectangle's commonest (canopy
 * in forest is no line across it). Only cuts of rectangles at least
 * SHARING a side are looked at.
 */
const STRIP_WIDE: usize = 12;
const STRIP_SHARE: f64 = 0.9;
const REACH: usize = 24;
const MIN_REACH: usize = 8;
const SHARING: usize = 32;
/** How far a drawn cut moves to a guide's edge, by default. */
pub const SNAP: usize = 16;

/** A rectangle's start and length on `axis` (0 x, 1 y). */
fn span(nd: &Node, axis: usize) -> (usize, usize) {
    if axis == 0 { (nd.x0, nd.w) } else { (nd.y0, nd.h) }
}

/** The shared strips of the cut along `axis` at `cut` of the rectangle `r` of `image`, from its start on the other axis. */
fn shared_strips(
    image: &Image,
    n: usize,
    r: (usize, usize, usize, usize),
    axis: usize,
    cut: usize,
    commonest: usize,
) -> Vec<(usize, usize)> {
    let (x0, y0, w, h) = r;
    let (along, lines) = if axis == 0 { (w, h) } else { (h, w) };
    let (left, right) = (cut.min(REACH), (along - cut).min(REACH));
    if w.min(h) < SHARING || left < MIN_REACH || right < MIN_REACH {
        return Vec::new();
    }
    let at = |t: usize, u: usize| if axis == 0 { image.at(x0 + u, y0 + t) } else { image.at(x0 + t, y0 + u) };
    // Each line's label if STRIP_SHARE of it on each side is that label.
    let mut count = vec![0usize; n];
    let label: Vec<Option<usize>> = (0..lines)
        .map(|t| {
            let side = |from: usize, to: usize, count: &mut Vec<usize>| {
                count.iter_mut().for_each(|c| *c = 0);
                (from..to).for_each(|u| count[at(t, u) as usize] += 1);
                let (l, c) = count.iter().enumerate().max_by_key(|(l, c)| (**c, std::cmp::Reverse(*l))).unwrap_or((0, &0));
                (*c as f64 >= STRIP_SHARE * (to - from) as f64).then_some(l)
            };
            let a = side(cut - left, cut, &mut count);
            let b = side(cut, cut + right, &mut count);
            a.filter(|l| Some(*l) == b && *l != commonest)
        })
        .collect();
    let mut out = Vec::new();
    let mut t = 0;
    while t < lines {
        let Some(l) = label[t] else {
            t += 1;
            continue;
        };
        let start = t;
        while t < lines && label[t] == Some(l) {
            t += 1;
        }
        if (2..=STRIP_WIDE).contains(&(t - start)) {
            out.push((start, t));
        }
    }
    out
}

/**
 * Marks the cuts in the subtree of `i` that lie on an edge of a guide
 * from above (`guides`, absolute, per axis), and the guides crossing each
 * rectangle.
 */
fn mark_snaps(nodes: &mut [Node], i: usize, guides: &mut [Vec<(usize, usize)>; 2]) {
    nodes[i].crossing = [0, 1].map(|axis| crossing(&guides[axis], span(&nodes[i], axis)));
    let Some((axis, cut)) = nodes[i].cut else { return };
    let at = span(&nodes[i], axis).0 + cut;
    nodes[i].snap = if guides[axis].iter().any(|g| g.0.abs_diff(at) <= 1) {
        Some(Edge::Lo)
    } else if guides[axis].iter().any(|g| g.1.abs_diff(at) <= 1) {
        Some(Edge::Hi)
    } else {
        None
    };
    let origin = span(&nodes[i], 1 - axis).0;
    let added: Vec<(usize, usize)> = nodes[i].shared.iter().map(|(lo, hi)| (origin + lo, origin + hi)).collect();
    let before = guides[1 - axis].len();
    guides[1 - axis].extend(added);
    for k in nodes[i].kids {
        mark_snaps(nodes, k, guides);
    }
    guides[1 - axis].truncate(before);
}

/** The centres of `guides` within (start, len), from start, sorted, those within SLACK of one before left out. */
fn crossing(guides: &[(usize, usize)], (start, len): (usize, usize)) -> Vec<usize> {
    let mut c: Vec<usize> = guides.iter().map(|g| (g.0 + g.1) / 2).filter(|c| *c >= start && *c < start + len).map(|c| c - start).collect();
    c.sort_unstable();
    c.dedup_by(|a, b| a.abs_diff(*b) <= SLACK);
    c
}

/** Guides' centres this close are one; a guide this far or farther from any of the other's costs as much. */
const SLACK: usize = 2;
const MISMATCH: usize = 16;

/**
 * How badly the guides `seen` crossing a training rectangle `from` long
 * fit the guides `wanted` crossing one `to` long: each placed as cuts
 * are, from the nearer end, a chamfer distance both ways.
 */
fn mismatch(seen: &[usize], from: usize, wanted: &[usize], to: usize) -> usize {
    let placed: Vec<usize> = seen.iter().filter_map(|p| if 2 * p <= from { Some(*p) } else { (p + to).checked_sub(from) }).collect();
    let near = |p: usize, of: &[usize]| of.iter().map(|q| q.abs_diff(p)).min().unwrap_or(MISMATCH).min(MISMATCH);
    wanted.iter().map(|p| near(*p, &placed)).sum::<usize>() + placed.iter().map(|p| near(*p, wanted)).sum::<usize>()
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
    nodes.push(Node {
        x0,
        y0,
        w,
        h,
        hist,
        cut: None,
        kids: [0, 0],
        crop,
        kind: 0,
        shared: Vec::new(),
        snap: None,
        crossing: [Vec::new(), Vec::new()],
    });
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
        let commonest = nodes[at].hist.iter().enumerate().max_by_key(|(l, c)| (**c, std::cmp::Reverse(*l))).map_or(0, |(l, _)| l);
        nodes[at].shared = shared_strips(image, n, (x0, y0, w, h), axis, cut, commonest);
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
    /**
     * Whether rules pass their shared strips down as guides; whether a
     * rule is chosen to fit the guides crossing its rectangle; how far a
     * snapping cut moves to a guide's edge (0 not at all).
     */
    pub shared: bool,
    pub matching: bool,
    pub snap: usize,
    /** Whether both sides of a cut draw with the same seed (W1's other try at lining them up). */
    pub same_seed: bool,
    /** Rules (nodes, if any) where the grammar stops: another model fills their rectangles (TEXTURE in the image, the kind in `kind`). */
    pub texture: Vec<bool>,
}

/** What a draw leaves where a texture rule's rectangle is, for another model to fill. */
pub const TEXTURE: u16 = 100;

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
    /** Each cell's place in that crop: its rule's rectangle scaled to the cell's. */
    pub at: Vec<(u16, u16)>,
    /** Each cell's rectangle's kind: its leaf's, or the texture rectangle's. */
    pub kind: Vec<u16>,
    pub rectangles: usize,
    /** Cuts drawn whose rule snaps, those with a guide's edge to snap to there, and those moved to it. */
    pub snaps: [usize; 3],
    /** Guides set by rules drawn. */
    pub guides: usize,
}

/** Guides a rectangle inherits: per axis, strips (lo, hi) in the crop's coordinates. */
type Guides = [Vec<(usize, usize)>; 2];

impl Grammar {
    /** The grammar of `crops` (labels below `n`), its rectangles in `kinds` kinds. */
    pub fn learn(crops: &[Image], n: usize, kinds: usize, seed: u64, contextual: bool) -> Grammar {
        let mut nodes = Vec::new();
        let roots: Vec<usize> = crops.iter().enumerate().map(|(i, c)| parse(c, n, i, (0, 0, c.w, c.h), &mut nodes)).collect();
        for &r in &roots {
            mark_snaps(&mut nodes, r, &mut [Vec::new(), Vec::new()]);
        }
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
        Grammar {
            n,
            kinds,
            nodes,
            roots,
            crops: crops.to_vec(),
            by_size,
            contextual,
            shared: false,
            matching: true,
            snap: SNAP,
            same_seed: false,
            texture: Vec::new(),
        }
    }

    /**
     * A rule for a rectangle of `kind`, w × h, in `context`: a node of that
     * kind of the nearest size (in that context, if one is within a
     * half-octave or two), chosen by `h`. With guides crossing it
     * (`wanted`, centres per axis), among the nodes of the nearest two sizes,
     * one whose guides in training fit them best.
     */
    fn rule(&self, context: usize, kind: usize, (w, hh): (usize, usize), wanted: &[Vec<usize>; 2], h: u64) -> Option<usize> {
        let (ow, oh) = (octaves(w), octaves(hh));
        let tries = if self.contextual && context > 0 { vec![(context, 2i32), (0, 12)] } else { vec![(0, 12)] };
        let guided = wanted.iter().any(|g| !g.is_empty());
        let mut found: Vec<usize> = Vec::new();
        for (c, radius) in tries {
            let mut first = None;
            for r in 0..=radius {
                if first.is_some_and(|f| r > f + guided as i32) {
                    break;
                }
                for dw in -r..=r {
                    for dh in -r..=r {
                        if dw.abs().max(dh.abs()) == r
                            && let Some(v) = self.by_size.get(&(c, kind, ow + dw, oh + dh))
                        {
                            found.extend(v);
                        }
                    }
                }
                if first.is_none() && !found.is_empty() {
                    first = Some(r);
                }
            }
            if !found.is_empty() {
                break;
            }
        }
        if found.is_empty() {
            return None;
        }
        found.sort_unstable();
        if guided {
            let cost = |e: usize| {
                let nd = &self.nodes[e];
                mismatch(&nd.crossing[0], nd.w, &wanted[0], w) + mismatch(&nd.crossing[1], nd.h, &wanted[1], hh)
            };
            let best = found.iter().map(|e| cost(*e)).min().unwrap_or(0);
            found.retain(|e| cost(*e) == best);
        }
        Some(found[(h % found.len() as u64) as usize])
    }

    /** The crop of `seed`, w × h like the training crops, over the box (x0, y0) bw × bh of it, alone. */
    pub fn sample(&self, seed: u64, (x0, y0, bw, bh): (usize, usize, usize, usize), leaves: Leaves) -> Drawn {
        let mut drawn = Drawn {
            image: Image::filled(bw, bh, 0),
            source: vec![0; bw * bh],
            at: vec![(0, 0); bw * bh],
            kind: vec![0; bw * bh],
            rectangles: 0,
            snaps: [0; 3],
            guides: 0,
        };
        let root = self.roots[(hash64(seed, 0, 0, 1) % self.roots.len() as u64) as usize];
        let (w, h) = (self.crops[0].w, self.crops[0].h);
        let mut guides: Guides = [Vec::new(), Vec::new()];
        let at = Rect { h: hash64(seed, 0, 0, 2), rule: Some(root), context: 0, kind: self.nodes[root].kind, r: (0, 0, w, h) };
        self.draw(at, &mut guides, (x0, y0, bw, bh), leaves, &mut drawn);
        drawn
    }

    fn draw(&self, at: Rect, guides: &mut Guides, bx: (usize, usize, usize, usize), leaves: Leaves, out: &mut Drawn) {
        let Rect { h, rule, context, kind, r } = at;
        let (rx, ry, rw, rh) = r;
        let (bx0, by0, bw, bh) = bx;
        if rx >= bx0 + bw || ry >= by0 + bh || rx + rw <= bx0 || ry + rh <= by0 {
            return;
        }
        out.rectangles += 1;
        // Fills the rectangle from rule `node`: each cell `label` of where it falls in the rule's.
        let cells = |out: &mut Drawn, node: &Node, label: &dyn Fn(usize, usize) -> u16| {
            for y in ry.max(by0)..(ry + rh).min(by0 + bh) {
                for x in rx.max(bx0)..(rx + rw).min(bx0 + bw) {
                    let (u, v) = (node.x0 + (x - rx) * node.w / rw, node.y0 + (y - ry) * node.h / rh);
                    let i = (y - by0) * bw + x - bx0;
                    (out.image.data[i], out.source[i], out.kind[i]) = (label(u, v), node.crop as u32, kind as u16);
                    out.at[i] = (u as u16, v as u16);
                }
            }
        };
        let wanted: [Vec<usize>; 2] = if self.shared && self.matching {
            [crossing(&guides[0], (rx, rw)), crossing(&guides[1], (ry, rh))]
        } else {
            [Vec::new(), Vec::new()]
        };
        let Some(e) = rule.or_else(|| self.rule(context, kind, (rw, rh), &wanted, h)) else { return };
        let node = &self.nodes[e];
        if self.texture.get(e).copied().unwrap_or(false) {
            cells(out, node, &|_, _| TEXTURE);
            return;
        }
        if let Some((axis, cut)) = node.cut {
            let (le, lt, start) = if axis == 0 { (node.w, rw, rx) } else { (node.h, rh, ry) };
            if lt >= 2 {
                let mut at = if cut * 2 <= le { cut } else { lt.saturating_sub(le - cut) }.clamp(1, lt - 1);
                if self.shared
                    && let Some(edge) = node.snap
                {
                    let edges = guides[axis].iter().map(|g| if edge == Edge::Lo { g.0 } else { g.1 });
                    let near = edges.filter(|p| *p > start && *p < start + lt).map(|p| p - start).min_by_key(|p| (p.abs_diff(at), *p));
                    out.snaps[0] += 1;
                    out.snaps[1] += near.is_some() as usize;
                    if let Some(p) = near.filter(|p| p.abs_diff(at) <= self.snap) {
                        out.snaps[2] += (p != at) as usize;
                        at = p;
                    }
                }
                // The rule's own strips, placed as its cut is, from the nearer end; only where none are inherited.
                let other = 1 - axis;
                let (ol, ot, ostart) = if other == 0 { (node.w, rw, rx) } else { (node.h, rh, ry) };
                let inherited = guides[other].iter().any(|g| g.1 > ostart && g.0 < ostart + ot);
                let before = guides[other].len();
                if self.shared && !inherited {
                    for &(lo, hi) in &node.shared {
                        let shift = |p: usize| if lo + hi <= ol { Some(p) } else { (p + ot).checked_sub(ol) };
                        if let (Some(lo), Some(hi)) = (shift(lo), shift(hi))
                            && lo >= 1
                            && hi < ot
                        {
                            guides[other].push((ostart + lo, ostart + hi));
                            out.guides += 1;
                        }
                    }
                }
                let (a, b) =
                    if axis == 0 { ((rx, ry, at, rh), (rx + at, ry, rw - at, rh)) } else { ((rx, ry, rw, at), (rx, ry + at, rw, rh - at)) };
                for (side, (kid, part)) in node.kids.iter().zip([a, b]).enumerate() {
                    let seed = hash64(h, if self.same_seed { 0 } else { side as i64 }, 0, 3);
                    let kid = Rect { h: seed, rule: None, context: 1 + kind * 2 + side, kind: self.nodes[*kid].kind, r: part };
                    self.draw(kid, guides, bx, leaves, out);
                }
                guides[other].truncate(before);
                return;
            }
        }
        // A leaf: the rule's pixels, scaled to the rectangle, or its commonest label.
        let crop = &self.crops[node.crop];
        let common = (0..self.n).max_by_key(|l| (node.hist[*l], std::cmp::Reverse(*l))).unwrap_or(0) as u16;
        cells(out, node, &|u, v| if leaves == Leaves::Pixels { crop.at(u, v) } else { common });
    }
}

/**
 * One in CUTS_EVERY rules of rectangles longer than MAX_LEAF is kept,
 * evenly through each (context, kind, size), and one in LEAVES_EVERY of
 * the rest, which a world draws as leaves, through each (kind, size); at
 * least one of each. In proportion to what training has of each, so a
 * rule is found about as often as its like was seen; few, to keep the
 * table small.
 */
pub const CUTS_EVERY: usize = 16;
pub const LEAVES_EVERY: usize = 256;
/** Labels that are built (roof, wall): a leaf's house. */
const BUILT: usize = 2;

/**
 * The rule table the learned world draws from (worlds/src/learned.rs
 * reads it): the roots, and of the other rules not under a `texture`
 * rule (the grammar stops there) some of each context, kind and size
 * (CUTS_EVERY, LEAVES_EVERY); a rule of a rectangle no longer than
 * MAX_LEAF is a leaf, which keeps only its ground (the commonest label
 * not built) and its house (the box of its built labels, if mostly
 * built and 3 m a side). After `TGR2`, every number a LEB128 varint:
 *
 * - labels, kinds, MAX_LEAF, the crop's side, rules, roots, and each root;
 * - each rule: w, h, kind, context (0 a root, else 1 + its parent's kind × 2 + which side),
 *   flags (1 cut, 2 along y, 4 snaps to a guide's low edge, 8 to its high one, 16 texture);
 * - a cut: its place, its sides' kinds, its shared strips (a count, then lo and hi of each),
 *   and the guides crossing it along x, then y (a count, then each);
 * - a leaf, not texture: its ground, and its house's x0, y0, x1, y1 (all 0 if none).
 */
pub fn export(g: &Grammar, texture: &[bool]) -> Vec<u8> {
    let mut context = vec![0usize; g.nodes.len()];
    let mut under = vec![false; g.nodes.len()];
    for (i, nd) in g.nodes.iter().enumerate() {
        if nd.cut.is_some() {
            for (side, k) in nd.kids.iter().enumerate() {
                context[*k] = 1 + nd.kind * 2 + side;
                under[*k] = under[i] || texture[i];
            }
        }
    }
    let leaf = |nd: &Node| nd.w.max(nd.h) <= MAX_LEAF;
    // Bins in index order, a few rules of each kept evenly through it.
    let mut bins: HashMap<(usize, usize, i32, i32), Vec<usize>> = HashMap::new();
    for (i, nd) in g.nodes.iter().enumerate().filter(|(i, _)| !under[*i]) {
        let c = if leaf(nd) { usize::MAX } else { context[i] };
        bins.entry((c, nd.kind, octaves(nd.w), octaves(nd.h))).or_default().push(i);
    }
    let mut keep = vec![false; g.nodes.len()];
    g.roots.iter().for_each(|r| keep[*r] = true);
    for (key, rules) in &bins {
        let most = rules.len().div_ceil(if key.0 == usize::MAX { LEAVES_EVERY } else { CUTS_EVERY });
        (0..most).for_each(|j| keep[rules[j * rules.len() / most]] = true);
    }
    let mut index = vec![u32::MAX; g.nodes.len()];
    let order: Vec<usize> = (0..g.nodes.len()).filter(|i| keep[*i]).collect();
    order.iter().enumerate().for_each(|(n, i)| index[*i] = n as u32);

    let mut out = b"TGR2".to_vec();
    let mut put = |v: usize| {
        let mut v = v as u64;
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    };
    [g.n, g.kinds, MAX_LEAF, g.crops[0].w, order.len(), g.roots.len()].into_iter().for_each(&mut put);
    g.roots.iter().for_each(|r| put(index[*r] as usize));
    for &i in &order {
        let nd = &g.nodes[i];
        let cut = nd.cut.filter(|_| !texture[i] && !leaf(nd));
        let snap = match nd.snap {
            Some(Edge::Lo) => 4,
            Some(Edge::Hi) => 8,
            None => 0,
        };
        let flags = cut.map_or(0, |(axis, _)| 1 | axis << 1) | snap | (texture[i] as usize) << 4;
        [nd.w, nd.h, nd.kind, context[i], flags].into_iter().for_each(&mut put);
        if let Some((_, at)) = cut {
            [at, g.nodes[nd.kids[0]].kind, g.nodes[nd.kids[1]].kind, nd.shared.len()].into_iter().for_each(&mut put);
            nd.shared.iter().flat_map(|(lo, hi)| [*lo, *hi]).for_each(&mut put);
            for c in &nd.crossing {
                put(c.len());
                c.iter().for_each(|p| put(*p));
            }
        } else if !texture[i] {
            leaf_of(g, nd).into_iter().for_each(|b| put(b as usize));
        }
    }
    out
}

/** A leaf's ground and house (see `export`). */
fn leaf_of(g: &Grammar, nd: &Node) -> [u8; 5] {
    let ground = (BUILT..g.n).max_by_key(|l| (nd.hist[*l], std::cmp::Reverse(*l))).filter(|l| nd.hist[*l] > 0).unwrap_or(PAVING);
    let crop = &g.crops[nd.crop];
    let (mut lo, mut hi, mut built) = ([usize::MAX; 2], [0; 2], 0);
    for y in 0..nd.h {
        for x in 0..nd.w {
            if (crop.at(nd.x0 + x, nd.y0 + y) as usize) < BUILT {
                (lo, hi, built) = ([lo[0].min(x), lo[1].min(y)], [hi[0].max(x + 1), hi[1].max(y + 1)], built + 1);
            }
        }
    }
    let (w, h) = (hi[0].saturating_sub(lo[0]), hi[1].saturating_sub(lo[1]));
    if w >= 3 && h >= 3 && 2 * built >= w * h {
        [ground as u8, lo[0] as u8, lo[1] as u8, hi[0] as u8, hi[1] as u8]
    } else {
        [ground as u8, 0, 0, 0, 0]
    }
}

/** Paving: a leaf's ground where all of it is built. */
const PAVING: usize = 3;

/** A rectangle to draw: its seed, rule if given, context, kind and extent. */
#[derive(Clone, Copy)]
struct Rect {
    h: u64,
    rule: Option<usize>,
    context: usize,
    kind: usize,
    r: (usize, usize, usize, usize),
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
    use crate::geometry::Crossings;

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

    /**
     * Towns of 128: a main street (1) down x 60..66, cross streets (1) 5
     * wide across it all every 30 from a place of their own, blocks of
     * roofs (2) on ground (0) between.
     */
    fn grids() -> Vec<Image> {
        (0..12)
            .map(|s| {
                let first = 4 + (s * 7) % 23;
                Image::new(
                    128,
                    128,
                    (0..128 * 128)
                        .map(|i| {
                            let (x, y) = (i % 128, i / 128);
                            let cross = y >= first && (y - first) % 30 < 5;
                            if (60..66).contains(&x) || cross {
                                1
                            } else if x % 12 < 8 && (y + 30 - first) % 30 > 7 && (y + 30 - first) % 30 < 27 {
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
    fn cross_streets_cross_with_shared_splits() {
        let mut g = Grammar::learn(&grids(), 3, 6, 1, true);
        assert!(g.nodes.iter().any(|nd| !nd.shared.is_empty()), "a cut with strips across it");
        let aligned = |g: &Grammar| {
            let all: Vec<Crossings> = (0..16).map(|s| Crossings::of(&g.sample(s, (0, 0, 128, 128), Leaves::Pixels).image, 1)).collect();
            Crossings::sum(&all).share()
        };
        let apart = aligned(&g);
        g.shared = true;
        let shared = aligned(&g);
        assert!(shared > 0.6 && shared > apart + 0.3, "aligned: shared {shared:.2}, apart {apart:.2}");
    }

    #[test]
    fn a_box_alone_is_the_same_box_of_the_whole_with_shared_splits() {
        let mut g = Grammar::learn(&grids(), 3, 6, 1, true);
        g.shared = true;
        for seed in 0..4 {
            let whole = g.sample(seed, (0, 0, 128, 128), Leaves::Pixels).image;
            for (x, y, w, h) in [(0, 0, 13, 9), (37, 51, 20, 7), (127, 127, 1, 1), (60, 3, 40, 100)] {
                assert_eq!(g.sample(seed, (x, y, w, h), Leaves::Pixels).image, whole.crop(x, y, w, h), "box {x} {y} {w} {h}");
            }
        }
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
