//! Wave function collapse on a few cells, some of whose neighbours are
//! already decided: hard constraints (which neighbour pairs a rule-set
//! allows) propagated by arc consistency, and each cell collapsed, most
//! constrained first, to a label drawn by the rule-set's weights: the
//! chance of each label given the neighbours decided (Contexts), or its
//! share times how much likelier each decided neighbour makes it
//! (pointwise mutual information), as a pairwise random field would.

use worldgen::noise::Rng;

/** The most labels a level has: domains are bit sets. */
pub const MAX_LABELS: usize = 64;

/** What a rule-set says of a pair of neighbours (a, b), a left of or below b. */
pub struct Adjacency {
    pub n: usize,
    /** For each a, the b allowed after it; for each b, the a allowed before it. */
    pub after: Vec<u64>,
    pub before: Vec<u64>,
    /** log P(a, b) / (P(a) P(b)), a * n + b. */
    pub pmi: Vec<f32>,
}

impl Adjacency {
    /** From pair counts (a * n + b), smoothed for the weights; the pairs `allowed` allowed. */
    pub fn new(n: usize, counts: &[f64], allowed: &dyn Fn(usize, usize) -> bool) -> Adjacency {
        let total: f64 = counts.iter().sum::<f64>().max(1.0);
        let (mut first, mut second) = (vec![0.0; n], vec![0.0; n]);
        for a in 0..n {
            for b in 0..n {
                first[a] += counts[a * n + b] / total;
                second[b] += counts[a * n + b] / total;
            }
        }
        let (mut after, mut before) = (vec![0u64; n], vec![0u64; n]);
        let mut pmi = vec![0.0f32; n * n];
        for a in 0..n {
            for b in 0..n {
                let c = counts[a * n + b];
                if allowed(a, b) {
                    after[a] |= 1 << b;
                    before[b] |= 1 << a;
                }
                let p = (c + 0.5) / (total + 0.5 * (n * n) as f64);
                pmi[a * n + b] = (p / ((first[a] + 1e-9) * (second[b] + 1e-9))).ln().clamp(-8.0, 8.0) as f32;
            }
        }
        Adjacency { n, after, before, pmi }
    }
}

/** A neighbour of a cell: another cell of the problem, or a label decided before. */
#[derive(Clone, Copy)]
pub enum Other {
    Cell(usize),
    Fixed(u16),
}

/**
 * A neighbour, which side of the cell it is on (DIRECTIONS), and the rule
 * of the pair: `first` if the cell is the pair's a (left of or below it).
 */
#[derive(Clone, Copy)]
pub struct Link<'a> {
    pub other: Other,
    pub side: usize,
    pub adjacency: &'a Adjacency,
    pub first: bool,
}

/** The sides of a cell: +x, +y, −x, −y. */
pub const DIRECTIONS: [(i64, i64); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

pub struct Cell<'a> {
    /** log share of each label in the cell's rule-set; −∞ where it has none. */
    pub prior: &'a [f32],
    pub links: Vec<Link<'a>>,
    /** How the rule-set weighs a label by the neighbours decided: by contexts seen, or (None) by its pairs alone. */
    pub contexts: Option<&'a Contexts>,
}

/**
 * How often each label was seen with each set of its four neighbours'
 * labels, and with each part of that set: the chance of a label given the
 * neighbours decided, as the data says, backing off to fewer neighbours
 * where the data is thin. Pairs alone, multiplied, make each neighbour
 * count as if it were the only one: labels that clump grow over the rest.
 */
#[derive(Default)]
pub struct Contexts {
    /** Per context, the count of each label, then their total. */
    table: std::collections::HashMap<u64, Vec<u32>>,
    n: usize,
}

/** Pseudo-counts of the coarser context in a finer one's chances. */
const BACKOFF: f64 = 2.0;

fn context_key(mask: usize, labels: &[u16; 4]) -> u64 {
    (0..4).filter(|d| mask & 1 << d != 0).fold(mask as u64, |k, d| k | (labels[d] as u64 + 1) << (4 + 12 * d))
}

impl Contexts {
    pub fn new(n: usize) -> Contexts {
        Contexts { table: Default::default(), n }
    }

    /** A cell of `label` seen with neighbours `labels` on the sides in `mask`, and so with each part of them. */
    pub fn add(&mut self, mask: usize, labels: &[u16; 4], label: u16) {
        for sub in 0..16 {
            if sub & mask == sub {
                let counts = self.table.entry(context_key(sub, labels)).or_insert_with(|| vec![0; self.n + 1]);
                counts[label as usize] += 1;
                counts[self.n] += 1;
            }
        }
    }

    fn total(&self, mask: usize, labels: &[u16; 4]) -> u32 {
        self.table.get(&context_key(mask, labels)).map_or(0, |c| c[self.n])
    }

    /** The chance of each label given the neighbours `labels` on the sides in `mask`, into `out`. */
    pub fn chances(&self, mask: usize, labels: &[u16; 4], out: &mut [f64]) {
        if mask == 0 {
            let counts = self.table.get(&0);
            let total = counts.map_or(0, |c| c[self.n]).max(1) as f64;
            for (a, o) in out.iter_mut().enumerate().take(self.n) {
                *o = counts.map_or(1.0, |c| c[a] as f64 / total);
            }
            return;
        }
        // Back off by dropping the neighbour whose loss leaves the most data.
        let coarser = (0..4).filter(|d| mask & 1 << d != 0).map(|d| mask & !(1 << d)).max_by_key(|m| self.total(*m, labels)).unwrap_or(0);
        self.chances(coarser, labels, out);
        if let Some(counts) = self.table.get(&context_key(mask, labels)) {
            let total = counts[self.n] as f64;
            for (a, o) in out.iter_mut().enumerate().take(self.n) {
                *o = (counts[a] as f64 + BACKOFF * *o) / (total + BACKOFF);
            }
        }
    }
}

fn allowed_for_other(link: &Link, domain: u64) -> u64 {
    let table = if link.first { &link.adjacency.after } else { &link.adjacency.before };
    bits(domain).fold(0, |m, a| m | table[a])
}

fn bits(mut set: u64) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        (set != 0).then(|| {
            let b = set.trailing_zeros() as usize;
            set &= set - 1;
            b
        })
    })
}

fn pmi(link: &Link, mine: usize, other: usize) -> f32 {
    let n = link.adjacency.n;
    if link.first { link.adjacency.pmi[mine * n + other] } else { link.adjacency.pmi[other * n + mine] }
}

/** Arc consistency from the cells in `queue`; false on a contradiction. */
fn propagate(cells: &[Cell], domain: &mut [u64], queue: &mut Vec<usize>) -> bool {
    while let Some(c) = queue.pop() {
        for link in &cells[c].links {
            if let Other::Cell(o) = link.other {
                let narrowed = domain[o] & allowed_for_other(link, domain[c]);
                if narrowed != domain[o] {
                    if narrowed == 0 {
                        return false;
                    }
                    domain[o] = narrowed;
                    queue.push(o);
                }
            }
        }
    }
    true
}

/** Gibbs sweeps after the collapse, where labels are weighed by contexts. */
const SWEEPS: usize = 4;

/**
 * Labels for `cells`, by WFC with randomness from `seed`; None on a
 * contradiction. With `strict` false, the pairs a rule-set never saw are
 * only unlikely, not forbidden: that never contradicts (the fallback).
 *
 * Collapsing cell by cell weighs each by the neighbours decided before
 * it, not after: labels that clump win the race. Where cells weigh by
 * contexts, Gibbs sweeps then redraw each from all its neighbours.
 */
pub fn solve(cells: &[Cell], seed: u64, strict: bool) -> Option<Vec<u16>> {
    let mut rng = Rng::new(seed);
    let mut domain: Vec<u64> =
        cells.iter().map(|c| c.prior.iter().enumerate().filter(|(_, p)| p.is_finite()).fold(0u64, |m, (l, _)| m | 1 << l)).collect();
    let labels = domain.clone();
    if strict {
        for (i, cell) in cells.iter().enumerate() {
            for link in &cell.links {
                if let Other::Fixed(f) = link.other {
                    let table = if link.first { &link.adjacency.before } else { &link.adjacency.after };
                    domain[i] &= table[f as usize];
                }
            }
        }
        if domain.contains(&0) {
            return None;
        }
        let mut queue: Vec<usize> = (0..cells.len()).collect();
        if !propagate(cells, &mut domain, &mut queue) {
            return None;
        }
    }
    let mut decided: Vec<Option<u16>> = vec![None; cells.len()];
    // Neighbours decided, of each cell.
    let mut known: Vec<usize> = cells.iter().map(|c| c.links.iter().filter(|l| matches!(l.other, Other::Fixed(_))).count()).collect();
    for _ in 0..cells.len() {
        // The most constrained cell: fewest labels left, then most neighbours decided.
        let next = (0..cells.len()).filter(|i| decided[*i].is_none()).min_by_key(|i| (domain[*i].count_ones(), usize::MAX - known[*i], *i));
        let Some(c) = next else { break };
        for link in &cells[c].links {
            if let Other::Cell(o) = link.other {
                known[o] += 1;
            }
        }
        let label = draw(&cells[c], domain[c], &decided, &mut rng);
        decided[c] = Some(label);
        domain[c] = 1 << label;
        if strict && !propagate(cells, &mut domain, &mut vec![c]) {
            return None;
        }
    }
    if cells.iter().any(|c| c.contexts.is_some()) {
        let mut order: Vec<usize> = (0..cells.len()).collect();
        for _ in 0..SWEEPS {
            for i in (1..order.len()).rev() {
                order.swap(i, rng.below(i as u32 + 1) as usize);
            }
            for &c in &order {
                // What all its neighbours allow it, as they are now.
                let mut options = labels[c];
                for link in cells[c].links.iter().filter(|_| strict) {
                    if let Some(b) = neighbour(link, &decided) {
                        options &= if link.first { link.adjacency.before[b as usize] } else { link.adjacency.after[b as usize] };
                    }
                }
                if options != 0 {
                    decided[c] = Some(draw(&cells[c], options, &decided, &mut rng));
                }
            }
        }
    }
    Some(decided.into_iter().map(|d| d.expect("every cell decided")).collect())
}

fn neighbour(link: &Link, decided: &[Option<u16>]) -> Option<u16> {
    match link.other {
        Other::Fixed(f) => Some(f),
        Other::Cell(o) => decided[o],
    }
}

/** A label for `cell` among `options`, drawn by its rule-set's weights given the neighbours decided. */
fn draw(cell: &Cell, options: u64, decided: &[Option<u16>], rng: &mut Rng) -> u16 {
    let mut weights = [0.0f32; MAX_LABELS];
    if let Some(contexts) = cell.contexts {
        let (mut mask, mut labels) = (0, [0u16; 4]);
        for link in &cell.links {
            if let Some(b) = neighbour(link, decided) {
                mask |= 1 << link.side;
                labels[link.side] = b;
            }
        }
        let mut chances = [0.0f64; MAX_LABELS];
        contexts.chances(mask, &labels, &mut chances);
        for a in bits(options) {
            // Labels the context never saw keep a little weight, so hard constraints alone decide what is possible.
            weights[a] = (chances[a] + 1e-6) as f32;
        }
    } else {
        let mut best = f32::MIN;
        for a in bits(options) {
            let mut s = cell.prior[a];
            for link in &cell.links {
                if let Some(b) = neighbour(link, decided) {
                    s += pmi(link, a, b as usize);
                }
            }
            weights[a] = s;
            best = best.max(s);
        }
        for a in bits(options) {
            weights[a] = (weights[a] - best).exp();
        }
    }
    let total: f32 = bits(options).map(|a| weights[a]).sum();
    let mut pick = rng.next() as f32 * total;
    for a in bits(options) {
        pick -= weights[a];
        if pick <= 0.0 {
            return a as u16;
        }
    }
    (63 - options.leading_zeros()) as u16
}
