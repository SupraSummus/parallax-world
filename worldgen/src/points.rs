//! Points: where things are, drawn top down, so a cell of any size
//! knows how many it holds without making them (docs/generators.md,
//! "Points").
//!
//! Cells are squares (or cubes, or segments) on the axes the process
//! spans: leaves `leaf` wide, and each cell twice as wide holding 2^D of
//! them, up to roots `root` wide. A root holds a Poisson number of points
//! of each mark; a cell's points go to its children one by one, each by
//! a child's weight times its leaves still free, so leaves take one each
//! while there is room; a leaf's stand where their mark has a chance.
//!
//! A cell weighs the chances at its corners: their mean times the leaves
//! it holds. So a cell with a point has a corner with a chance, and the
//! child at that corner has it too, down to a leaf: a point never comes
//! down to where its mark has none. And a cell's counts are its
//! children's, added, by construction: far forms read the things that
//! stand. It costs a cell its corners, which are sites of every level
//! whose sites are at most as far apart.

use std::cell::RefCell;
use std::rc::Rc;

use crate::cost;
use crate::js;
use crate::noise::{Fixed, Fx, Rng, hash_text, hash64, unit};
use crate::plan;

/** The most marks a process draws: kinds of things, each with chances of its own. */
pub const MARKS: usize = 4;

/** By mark, the chance a leaf at a point holds a thing of it: summed, at most one. */
pub type Chances = [Fx; MARKS];

/** A count of points by mark. */
pub type Counts = [u32; MARKS];

const DOCS: &str = "docs/generators.md";

/** A point: where it stands (lattice units; 0 along the axes the process doesn't span), its mark, and its leaf's corner and its number there. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub at: [i64; 3],
    pub mark: usize,
    pub leaf: [i64; 3],
    pub nth: u32,
}

/** Positions a leaf tries for a point, at random, before it stands at the corner whose chance let it in. */
const TRIES: u32 = 16;

/** Slots each cache keeps values in, the last made in each: enough for a host tile's cells at any level. */
const SLOTS: usize = 1 << 15;

/** Values kept by key, the last made in each slot. */
struct Slots<K, V>(RefCell<Vec<Option<(K, V)>>>);

impl<K: Copy + PartialEq, V: Clone> Slots<K, V> {
    fn new(slots: usize) -> Self {
        Slots(RefCell::new(vec![None; slots]))
    }

    fn get(&self, slot: usize, key: K, make: impl FnOnce() -> V) -> V {
        if let Some((k, v)) = &self.0.borrow()[slot]
            && *k == key
        {
            return v.clone();
        }
        let v = make();
        self.put(slot, key, v.clone());
        v
    }

    fn put(&self, slot: usize, key: K, v: V) {
        self.0.borrow_mut()[slot] = Some((key, v));
    }
}

/**
 * A hierarchical point process: a pure function of its seed and each
 * cell, whichever cells are asked first, with every cell's counts the sum
 * of its children's.
 */
pub struct Points {
    name: &'static str,
    seed: u64,
    /** The axes its cells span. */
    spans: [bool; 3],
    /** Those axes, in order, and how many. */
    axes: [usize; 3],
    dims: u32,
    /** Leaves' and roots' width, lattice units, powers of two. */
    pub leaf: i64,
    pub root: i64,
    marks: usize,
    chances: Box<dyn Fn([i64; 3]) -> Chances>,
    corners: Slots<[i64; 3], Chances>,
    /** By cell, its counts. */
    counted: Slots<(i64, [i64; 3]), Counts>,
}

impl Points {
    /**
     * Points of `marks` marks in cells on `axes`, from leaves `leaf` wide
     * to roots `root` wide, with `chances` at each point (lattice units;
     * 0 along the other axes): by mark, the chance a leaf there holds a
     * point of it. Where they vary within a root, the points follow its
     * cells' corners, so roots want to be no wider than they vary over.
     */
    pub fn new(
        name: &'static str,
        seed: u64,
        axes: &[usize],
        leaf: i64,
        root: i64,
        marks: usize,
        chances: impl Fn([i64; 3]) -> Chances + 'static,
    ) -> Points {
        assert!(
            leaf > 0 && (leaf as u64).is_power_of_two() && (root as u64).is_power_of_two() && root >= leaf,
            "points {name}: leaves and roots are powers of two wide, roots at least leaves; see \"Points\" in {DOCS}"
        );
        assert!(marks <= MARKS && !axes.is_empty(), "points {name}: 1 to 3 axes, at most {MARKS} marks");
        let spans = [0, 1, 2].map(|a| axes.contains(&a));
        Points {
            name,
            seed: hash_text(name, seed),
            spans,
            axes: std::array::from_fn(|i| axes.get(i).copied().unwrap_or(0)),
            dims: spans.iter().filter(|s| **s).count() as u32,
            leaf,
            root,
            marks,
            chances: Box::new(chances),
            corners: Slots::new(SLOTS),
            counted: Slots::new(SLOTS),
        }
    }

    /** Whether its cells span each axis. */
    pub fn spans(&self) -> [bool; 3] {
        self.spans
    }

    /** The corner of the cell `size` wide holding `at`: a multiple of `size` along the axes spanned, 0 along the others. */
    fn cell(&self, size: i64, at: [i64; 3]) -> [i64; 3] {
        [0, 1, 2].map(|a| if self.spans[a] { at[a].div_euclid(size) * size } else { 0 })
    }

    /** The corners of the cell `size` wide at `corner` (or its children's, with half its width): 2^D of them. */
    fn corners_of(&self, size: i64, corner: [i64; 3]) -> impl Iterator<Item = [i64; 3]> + '_ {
        (0..1u32 << self.dims).map(move |c| {
            let mut at = corner;
            for (bit, &axis) in self.axes[..self.dims as usize].iter().enumerate() {
                at[axis] += size * ((c >> bit) & 1) as i64;
            }
            at
        })
    }

    /** The chances at a point, by mark: a leaf there holds that many points of each, on average. */
    fn chances(&self, at: [i64; 3]) -> Chances {
        // The same whoever asks: a layer reads dithered plans at a point of its sites' footprint.
        plan::where_asked(|| (self.chances)(at))
    }

    /**
     * The slot of the cell `size` wide at `corner`: its index along each
     * axis spanned, modulo a power of two, and its width's parity, so the
     * cells a tile reads lie side by side, as the tile's columns do.
     */
    fn slot(&self, size: i64, corner: [i64; 3]) -> usize {
        let (bits, shift) = ((SLOTS.trailing_zeros() - 1) / self.dims, size.trailing_zeros());
        let mut slot = (shift as usize & 1) << (SLOTS.trailing_zeros() - 1);
        for (n, &axis) in self.axes[..self.dims as usize].iter().enumerate() {
            slot |= ((corner[axis] >> shift) as usize & ((1 << bits) - 1)) << (n as u32 * bits);
        }
        slot
    }

    /** The chances at a point, kept. */
    fn chances_at(&self, at: [i64; 3]) -> Chances {
        self.corners.get(self.slot(self.leaf, at), at, || cost::plan(self.name, || self.chances(at)))
    }

    /** By mark, the weight of the cell `size` wide at `corner`: its corners' chances, added (fixed point), times the leaves it holds. */
    fn weights(&self, size: i64, corner: [i64; 3]) -> [Fixed; MARKS] {
        let leaves = (size / self.leaf).pow(self.dims);
        let mut sum = [0; MARKS];
        for at in self.corners_of(size, corner) {
            let chances = self.chances_at(at);
            (0..self.marks).for_each(|m| sum[m] += chances[m].max(Fx::ZERO).to_bits());
        }
        sum.map(|w| w * leaves)
    }

    /**
     * How many points of each mark the cell `size` wide at `corner`
     * holds (a power of two, from leaves up; wider than roots, the roots'
     * in it added).
     */
    pub fn counts(&self, size: i64, corner: [i64; 3]) -> Counts {
        cost::lookup();
        assert!(size >= self.leaf && (size as u64).is_power_of_two(), "points {}: no cells {size} wide", self.name);
        self.kept(size, corner)
    }

    /** `counts`, kept. */
    fn kept(&self, size: i64, corner: [i64; 3]) -> Counts {
        self.counted.get(self.slot(size, corner), (size, corner), || {
            if size > self.root {
                let mut counts = [0; MARKS];
                for root in self.cells_in(self.root, corner, corner.map(|v| v + size)) {
                    let c = self.kept(self.root, root);
                    (0..MARKS).for_each(|m| counts[m] += c[m]);
                }
                return counts;
            }
            if size == self.root {
                let weights = self.weights(size, corner);
                return std::array::from_fn(|m| {
                    let mean = weights[m] as f64 / (1u64 << (16 + self.dims)) as f64;
                    let h = hash64(self.seed, corner[0], corner[1], corner[2] ^ ((m as i64) << 56));
                    poisson(mean, unit((h >> 32) as u32))
                });
            }
            // Its siblings' too: they are asked next.
            let (parent, mut counts) = (self.cell(2 * size, corner), [0; MARKS]);
            for (child, split) in self.corners_of(size, parent).zip(self.split(2 * size, parent)) {
                self.counted.put(self.slot(size, child), (size, child), split);
                if child == corner {
                    counts = split;
                }
            }
            counts
        })
    }

    /** The counts of the children of the cell `size` wide at `corner`, in the order of `corners_of`: its points, each to a child by their weights. */
    fn split(&self, size: i64, corner: [i64; 3]) -> [Counts; 8] {
        let counts = self.kept(size, corner);
        let mut weights = [[0; MARKS]; 8];
        for (w, child) in weights.iter_mut().zip(self.corners_of(size / 2, corner)) {
            *w = self.weights(size / 2, child);
        }
        // Leaves free in each child: where none it may go to has any, by its weight alone.
        let mut free = [(size / 2 / self.leaf).pow(self.dims); 8];
        let mut split = [[0; MARKS]; 8];
        for m in 0..self.marks {
            for j in 0..counts[m] {
                let mut weight: [Fixed; 8] = std::array::from_fn(|i| weights[i][m] * free[i]);
                if weight.iter().all(|w| *w == 0) {
                    weight = std::array::from_fn(|i| weights[i][m]);
                }
                let total: Fixed = weight.iter().sum();
                assert!(
                    total > 0,
                    "points {}: the cell {size} wide at {corner:?} holds a point of mark {m}, but its children weigh nothing",
                    self.name
                );
                let h = hash64(self.seed, corner[0] ^ size, corner[1] ^ ((m as i64) << 48), corner[2] ^ ((j as i64) << 32));
                let mut pick = ((h as u128 * total as u128) >> 64) as Fixed;
                let child = weight.iter().position(|w| {
                    pick -= w;
                    pick < 0
                });
                let child = child.expect("within the total");
                split[child][m] += 1;
                free[child] = (free[child] - 1).max(0);
            }
        }
        split
    }

    /** The corners of the cells `size` wide meeting [min, max) along the axes spanned. */
    fn cells_in(&self, size: i64, min: [i64; 3], max: [i64; 3]) -> Vec<[i64; 3]> {
        let range = |a: usize| {
            if self.spans[a] { (min[a].div_euclid(size), (max[a] - 1).div_euclid(size)) } else { (0, 0) }
        };
        let [(i0, i1), (j0, j1), (k0, k1)] = [0, 1, 2].map(range);
        let mut cells = Vec::new();
        for i in i0..=i1 {
            for j in j0..=j1 {
                for k in k0..=k1 {
                    cells.push([i, j, k].map(|v| v * size));
                }
            }
        }
        cells
    }

    /**
     * The points of the leaves meeting [min, max) along the axes spanned
     * (lattice units): each where it stands, evenly where its mark has a
     * chance in its leaf, or at the corner of its leaf whose chance let
     * it in, if TRIES positions found none.
     */
    pub fn points_in(&self, min: [i64; 3], max: [i64; 3]) -> Vec<Spot> {
        let mut spots = Vec::new();
        for leaf in self.cells_in(self.leaf, min, max) {
            let counts = self.kept(self.leaf, leaf);
            for (mark, &count) in counts.iter().enumerate() {
                for nth in 0..count {
                    let mut random = Rng::new(hash64(self.seed ^ ((mark as u64) << 40 | nth as u64), leaf[0], leaf[1], leaf[2]));
                    let tried = (0..TRIES).find_map(|_| {
                        let at = [0, 1, 2].map(|a| if self.spans[a] { leaf[a] + random.below(self.leaf as u32) as i64 } else { 0 });
                        (self.chances(at)[mark] > Fx::ZERO).then_some(at)
                    });
                    let at = tried.unwrap_or_else(|| {
                        self.corners_of(self.leaf, leaf).find(|c| self.chances_at(*c)[mark] > Fx::ZERO).expect("a corner let it in")
                    });
                    spots.push(Spot { at, mark, leaf, nth });
                }
            }
        }
        spots
    }
}

/** Cells along each axis of a blur's blocks, and blocks along each axis it keeps: a host tile's. */
const BLOCK: i64 = 16;
const BLOCKS: i64 = 8;

/** By mark, a weight. */
pub type Weights = [f64; MARKS];

/**
 * The counts of cells `size` wide around a cell, by mark, each times the
 * weights of its offset (in cells), added: what a far form reads of the
 * things around a column. For points on two axes. It works out a block
 * of cells at once, reading each cell once, not once per cell around.
 */
pub struct Blur {
    points: Rc<Points>,
    size: i64,
    taps: Vec<([i64; 2], Weights)>,
    blocks: Slots<[i64; 2], Rc<[Weights]>>,
}

impl Blur {
    /** Of `points`' cells `size` wide, with `taps`: offsets in cells, each with its weights. */
    pub fn new(points: Rc<Points>, size: i64, taps: Vec<([i64; 2], Weights)>) -> Blur {
        assert!(points.dims == 2, "points {}: blurs are of points on two axes", points.name);
        Blur { points, size, taps, blocks: Slots::new((BLOCKS * BLOCKS) as usize) }
    }

    /** At the cell `size` wide whose corner is `at`, along the points' two axes. */
    pub fn at(&self, at: [i64; 2]) -> Weights {
        let cell = at.map(|v| v.div_euclid(self.size));
        let block = cell.map(|c| c.div_euclid(BLOCK));
        let slot = (block[0].rem_euclid(BLOCKS) * BLOCKS + block[1].rem_euclid(BLOCKS)) as usize;
        let [u, v] = [0, 1].map(|a| cell[a].rem_euclid(BLOCK));
        self.blocks.get(slot, block, || self.block(block))[(u * BLOCK + v) as usize]
    }

    /** The values of a block's cells, from the counts of those it reaches, read once each. */
    fn block(&self, block: [i64; 2]) -> Rc<[Weights]> {
        let reach = self.taps.iter().flat_map(|(o, _)| o.map(i64::abs)).max().unwrap_or(0);
        let (first, side) = (block.map(|b| b * BLOCK - reach), BLOCK + 2 * reach);
        let [a, b] = [self.points.axes[0], self.points.axes[1]];
        let counts: Vec<Counts> = (0..side * side)
            .map(|i| {
                let mut corner = [0; 3];
                (corner[a], corner[b]) = ((first[0] + i / side) * self.size, (first[1] + i % side) * self.size);
                self.points.counts(self.size, corner)
            })
            .collect();
        // Each cell holding points adds to the cells it reaches, in the same order for every block.
        let mut sums = vec![[0.0; MARKS]; (BLOCK * BLOCK) as usize];
        for (i, n) in counts.iter().enumerate().filter(|(_, n)| **n != [0; MARKS]) {
            let (cu, cv) = (i as i64 / side - reach, i as i64 % side - reach);
            for ([du, dv], weights) in &self.taps {
                let (u, v) = (cu - du, cv - dv);
                if (0..BLOCK).contains(&u) && (0..BLOCK).contains(&v) {
                    let sum = &mut sums[(u * BLOCK + v) as usize];
                    (0..MARKS).for_each(|m| sum[m] += weights[m] * n[m] as f64);
                }
            }
        }
        sums.into()
    }
}

/** A Poisson number of `mean` at `u`, a fraction of 1: the least k whose cumulative chance passes it. */
fn poisson(mean: f64, u: f64) -> u32 {
    let (mut k, mut p) = (0, js::exp(-mean));
    let mut cumulative = p;
    while u >= cumulative && k < 1000 && p > 0.0 {
        k += 1;
        p *= mean / k as f64;
        cumulative += p;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::fx;

    /** Points in a square 4096 wide of a disc of chance ¼ in a field of ⅛, a mark each, a clearing round the origin. */
    fn points(seed: u64) -> Points {
        Points::new("test", seed, &[0, 2], 16, 64, 2, |[x, _, z]| {
            let r2 = x * x + z * z;
            let mut c = [Fx::ZERO; MARKS];
            if r2 > 300 * 300 {
                c[0] = fx(0.125);
            } else if r2 > 100 * 100 {
                c[1] = fx(0.25);
            }
            c
        })
    }

    #[test]
    fn cells_hold_their_childrens_points_whichever_is_asked_first() {
        let (a, b) = (points(3), points(3));
        // Asked in opposite orders, cell by cell from roots down, and leaves first.
        for size in [256, 128, 64, 32, 16] {
            for corner in a.cells_in(size, [-1024, 0, -1024], [1024, 1, 1024]) {
                let counts = a.counts(size, corner);
                if size > 16 {
                    let mut sum = [0; MARKS];
                    for c in a.corners_of(size / 2, corner) {
                        let child = a.counts(size / 2, c);
                        (0..MARKS).for_each(|m| sum[m] += child[m]);
                    }
                    assert_eq!(counts, sum, "{size} at {corner:?}");
                }
            }
        }
        let square = |a: &Points, size| a.cells_in(size, [-1024, 0, -1024], [1024, 1, 1024]);
        let roots: u32 = square(&a, 64).iter().map(|c| a.counts(64, *c).iter().sum::<u32>()).sum();
        assert_eq!(b.points_in([-1024, 0, -1024], [1024, 1, 1024]).len() as u32, roots, "the leaves hold the roots' points");
    }

    #[test]
    fn points_stand_where_their_mark_has_a_chance_as_often_as_it_is() {
        let (mut field, mut disc) = (0, 0);
        for seed in 0..8 {
            let p = points(seed);
            for spot in p.points_in([-2048, 0, -2048], [2048, 1, 2048]) {
                let r2 = spot.at[0] * spot.at[0] + spot.at[2] * spot.at[2];
                match spot.mark {
                    0 => assert!(r2 > 300 * 300, "{spot:?} in the disc"),
                    _ => assert!(r2 > 100 * 100 && r2 <= 300 * 300, "{spot:?} off the disc"),
                }
                if spot.mark == 0 { field += 1 } else { disc += 1 }
            }
        }
        // Expected: (4096² − π 300²) / 256 · ⅛ and π (300² − 100²) / 256 · ¼ a seed.
        let expect = |area: f64, chance: f64| 8.0 * area / 256.0 * chance;
        let pi = std::f64::consts::PI;
        let (f, d) = (expect(4096.0 * 4096.0 - pi * 9e4, 0.125), expect(pi * 8e4, 0.25));
        assert!((field as f64 / f - 1.0).abs() < 0.02, "{field} in the field, expected {f}");
        assert!((disc as f64 / d - 1.0).abs() < 0.1, "{disc} on the disc, expected {d}");
    }

    #[test]
    fn a_blur_is_its_cells_counts_weighed_wherever_its_blocks_fall() {
        let p = Rc::new(points(5));
        let taps: Vec<_> =
            [[-2, 1], [0, 0], [1, -1], [3, 2]].into_iter().zip([[1.0; MARKS], [0.5; MARKS], [2.0; MARKS], [0.25; MARKS]]).collect();
        let blur = Blur::new(p.clone(), 32, taps.clone());
        // Across blocks' edges, and back after other blocks took their slots.
        for [x, z] in [[-640, 480], [-608, 480], [-640, 512], [4000, -4000], [-640, 480]] {
            let mut sum = [0.0; MARKS];
            for ([dx, dz], w) in &taps {
                let n = p.counts(32, [x + dx * 32, 0, z + dz * 32]);
                (0..MARKS).for_each(|m| sum[m] += w[m] * n[m] as f64);
            }
            assert_eq!(blur.at([x, z]), sum, "at {x}, {z}");
        }
    }
}
