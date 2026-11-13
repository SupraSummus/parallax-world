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
//! A leaf weighs the chances at its corners, a cell its leaves. So a cell
//! with a point has a leaf with a corner with a chance, as has the child
//! holding that leaf, down to it: a point never comes down to where its
//! mark has none. And a cell's counts are its children's, added, by
//! construction: far forms read the things that stand. It costs a cell
//! its leaves' corners, sites of the levels whose sites are at most a
//! leaf apart.

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
    /** By leaf holding any, its points, placed: each costs reading its chances up to TRIES times. */
    placed: Slots<[i64; 3], Rc<[Spot]>>,
}

impl Points {
    /**
     * Points of `marks` marks in cells on `axes`, from leaves `leaf` wide
     * to roots `root` wide, with `chances` at each point (lattice units;
     * 0 along the other axes): by mark, the chance a leaf there holds a
     * point of it. Where they vary within a leaf, the points follow its
     * corners, so leaves want to be no wider than they vary over.
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
            placed: Slots::new(SLOTS),
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
        self.corners.get(self.slot(self.leaf, at), at, || cost::plan(self.name, self, at, || self.chances(at)))
    }

    /** By mark, the weight of the cell `size` wide at `corner`: its leaves' corners' chances, added (fixed point). */
    fn weights(&self, size: i64, corner: [i64; 3]) -> [Fixed; MARKS] {
        let mut sum = [0; MARKS];
        for leaf in self.cells_in(self.leaf, corner, corner.map(|v| v + size)) {
            for at in self.corners_of(self.leaf, leaf) {
                let chances = self.chances_at(at);
                (0..self.marks).for_each(|m| sum[m] += chances[m].max(Fx::ZERO).to_bits());
            }
        }
        sum
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
                // Its own hash: keyed by its corner XOR its width, the first pick of a root's child shared the root's count's,
                // and sent the child's first point to its last leaf wherever the root held any.
                let h = hash64(hash64(self.seed, size, m as i64, j as i64), corner[0], corner[1], corner[2]);
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
            if counts != [0; MARKS] {
                spots.extend_from_slice(&self.placed.get(self.slot(self.leaf, leaf), leaf, || self.place(leaf, counts)));
            }
        }
        spots
    }

    /** The points of the leaf at `leaf`, `counts` of them by mark, placed. */
    fn place(&self, leaf: [i64; 3], counts: Counts) -> Rc<[Spot]> {
        let mut spots = Vec::new();
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
        spots.into()
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

/**
 * The rooms of the things at a process's points: its power diagram
 * (Aurenhammer). A place is the room of the thing whose power there,
 * |p − a|² − weight, is least, if it reaches it; each wall loses half a
 * gap to either side. Weights are of a point alone and each pair's gap is
 * drawn from the pair, so both sides of a wall agree on it. Whose room a
 * place is needs only the points within √(reach² + most) of it: one
 * whose power beats a thing within reach stands that near.
 */
pub struct Rooms {
    points: Rc<Points>,
    reach: i64,
    most: i64,
    /** The least and most gap between two rooms (units), each pair's drawn between. */
    gaps: [i64; 2],
    seed: u64,
    weight: Box<dyn Fn(&Spot) -> i64>,
    /** By block `reach` wide, the points that may own its places: BLOCKS along each axis spanned, as a host tile's. */
    near: Slots<[i64; 3], Rc<[Spot]>>,
    /** By block, the points in it, for `within`: twice as many blocks along each axis, as it reads those 3 around. */
    blocks: Slots<[i64; 3], Rc<[Spot]>>,
}

impl Rooms {
    /** The rooms of `points`' things, reaching `reach`, weighed by `weight`, 0 to `most`, `gaps` apart. */
    pub fn new(points: Rc<Points>, reach: i64, most: i64, gaps: [i64; 2], weight: impl Fn(&Spot) -> i64 + 'static) -> Rooms {
        assert!(reach > 0 && most >= 0 && gaps[0] <= gaps[1], "points {}: rooms reach somewhere, weights and gaps are ranges", points.name);
        let seed = hash_text("rooms", points.seed);
        let slots = BLOCKS.pow(points.dims) as usize;
        let blocks = Slots::new((2 * BLOCKS).pow(points.dims) as usize);
        Rooms { points, reach, most, gaps, seed, weight: Box::new(weight), near: Slots::new(slots), blocks }
    }

    /** How far a thing beating another within reach of a place may stand from it: √(reach² + most), rounded up. */
    fn rival(&self) -> i64 {
        (self.reach * self.reach + self.most).isqrt() + 1
    }

    fn weight(&self, spot: &Spot) -> i64 {
        let w = (self.weight)(spot);
        assert!(
            (0..=self.most).contains(&w),
            "points {}: a weight of {w}, not 0 to {}; see \"Rooms\" in {DOCS}",
            self.points.name,
            self.most
        );
        w
    }

    /** `at` on the axes the points span, 0 on the others. */
    fn on_axes(&self, at: [i64; 3]) -> [i64; 3] {
        [0, 1, 2].map(|a| if self.points.spans[a] { at[a] } else { 0 })
    }

    /** The points within `within` of `at` (on the axes), of the blocks `reach` wide they may be in. */
    fn within(&self, at: [i64; 3], within: i64) -> Vec<Spot> {
        let range =
            |a: usize| if self.points.spans[a] { [at[a] - within, at[a] + within].map(|v| v.div_euclid(self.reach)) } else { [0, 0] };
        let [i, j, k] = [0, 1, 2].map(range);
        let mut spots = Vec::new();
        for block in (i[0]..=i[1]).flat_map(|i| (j[0]..=j[1]).flat_map(move |j| (k[0]..=k[1]).map(move |k| [i, j, k]))) {
            spots.extend(self.block(block).iter().filter(|s| squared(s.at, at) <= within * within));
        }
        spots
    }

    /** The points in a block, kept: of the leaves meeting it or a leaf below, as a point may stand at its leaf's far corner. */
    fn block(&self, block: [i64; 3]) -> Rc<[Spot]> {
        let axes = &self.points.axes[..self.points.dims as usize];
        let slot = axes.iter().fold(0, |slot, &a| slot * 2 * BLOCKS + block[a].rem_euclid(2 * BLOCKS)) as usize;
        self.blocks.get(slot, block, || {
            let (min, max) = (block.map(|b| b * self.reach), block.map(|b| (b + 1) * self.reach));
            let mut spots = self.points.points_in(min.map(|v| v - self.points.leaf), max);
            spots.retain(|s| (0..3).all(|a| !self.points.spans[a] || (min[a]..max[a]).contains(&s.at[a])));
            spots.into()
        })
    }

    /** The gap between the rooms of `a` and `b`, `distance` apart: drawn from the pair, in either order; at most half their distance, so their walls stay between them. */
    fn gap(&self, a: &Spot, b: &Spot, distance: f64) -> f64 {
        let key = |s: &Spot| (s.at, s.mark, s.nth);
        let (a, b) = if key(a) <= key(b) { (a, b) } else { (b, a) };
        let h = hash64(self.seed, a.at[0], a.at[1], a.at[2] ^ ((a.mark as i64) << 40 | (a.nth as i64) << 48));
        let u = unit(hash64(h, b.at[0], b.at[1], b.at[2] ^ ((b.mark as i64) << 40 | (b.nth as i64) << 48)) as u32);
        (self.gaps[0] as f64 + u * (self.gaps[1] - self.gaps[0]) as f64).min(distance / 2.0)
    }

    /**
     * The room of the thing of `mark` at `at`: walled off from each that
     * may beat it within its reach. Things at one point share its room.
     */
    pub fn room(&self, at: [i64; 3], mark: usize) -> Room {
        let at = self.on_axes(at);
        let others = self.within(at, self.reach + self.rival());
        let own = *others
            .iter()
            .filter(|s| s.at == at && s.mark == mark)
            .min_by_key(|s| (s.leaf, s.nth))
            .unwrap_or_else(|| panic!("points {}: none of mark {mark} at {at:?}", self.points.name));
        let w = self.weight(&own);
        let mut room = Room { at: at.map(|v| v as f64), walls: Vec::new() };
        for other in others.iter().filter(|s| s.at != at) {
            let d = [0, 1, 2].map(|i| (other.at[i] - at[i]) as f64);
            let distance = js::hypot(&d);
            // Where the two powers are equal, along the line between them.
            let equal = (distance * distance + (w - self.weight(other)) as f64) / (2.0 * distance);
            room.walls.push((d.map(|v| v / distance), equal - self.gap(&own, other, distance) / 2.0));
        }
        room
    }

    /** The thing whose room `at` is in, gaps aside, if one reaches it; of equal powers, the first point. */
    pub fn owner(&self, at: [i64; 3]) -> Option<Spot> {
        let at = self.on_axes(at);
        let block = at.map(|v| v.div_euclid(self.reach));
        let axes = &self.points.axes[..self.points.dims as usize];
        let slot = axes.iter().fold(0, |slot, &a| slot * BLOCKS + block[a].rem_euclid(BLOCKS)) as usize;
        let near = self.near.get(slot, block, || {
            let (centre, half) = (block.map(|b| b * self.reach + self.reach / 2), self.reach / 2 + 1);
            self.within(self.on_axes(centre), half * 2 + self.rival()).into()
        });
        let power = |s: &Spot| squared(s.at, at) - self.weight(s);
        let owner = near.iter().min_by_key(|s| (power(s), s.at, s.mark, s.nth))?;
        (squared(owner.at, at) <= self.reach * self.reach).then_some(*owner)
    }
}

/** The squared distance between two points. */
fn squared(a: [i64; 3], b: [i64; 3]) -> i64 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/** A thing's room (`Rooms`): on its side of a wall between it and each thing that may beat it. */
pub struct Room {
    at: [f64; 3],
    /** Unit normals towards each neighbour, and how far along it from `at` its wall stands. */
    walls: Vec<([f64; 3], f64)>,
}

impl Room {
    /** The room of a thing alone: everywhere. */
    pub fn everywhere() -> Room {
        Room { at: [0.0; 3], walls: Vec::new() }
    }

    /** How far `p` is inside the room: from the nearest wall, negative outside. */
    pub fn margin(&self, p: [f64; 3]) -> f64 {
        let along = |n: &[f64; 3]| (0..3).map(|i| n[i] * (p[i] - self.at[i])).sum::<f64>();
        self.walls.iter().map(|(n, wall)| wall - along(n)).fold(f64::INFINITY, f64::min)
    }

    /** How much of the way from `from` to `to` keeps at least `margin` inside the room: a fraction, 0 if `from` doesn't. */
    pub fn reach(&self, from: [f64; 3], to: [f64; 3], margin: f64) -> f64 {
        let step = [0, 1, 2].map(|i| to[i] - from[i]);
        let mut t: f64 = 1.0;
        for (n, wall) in &self.walls {
            let towards: f64 = (0..3).map(|i| n[i] * step[i]).sum();
            let left = wall - margin - (0..3).map(|i| n[i] * (from[i] - self.at[i])).sum::<f64>();
            if towards > 0.0 {
                t = t.min(left / towards);
            }
        }
        t.max(0.0)
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
    use crate::noise::{fx, hash};

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
    fn a_roots_leaves_hold_alike_where_their_chances_are() {
        // Every leaf of a root a chance of ⅛, over 4096² at four seeds: about 2048 points at each leaf's place in its root.
        let mut at = [0.0; 16];
        for seed in 0..4 {
            let p = Points::new("even", seed, &[0, 2], 16, 64, 1, |_| [fx(0.125), Fx::ZERO, Fx::ZERO, Fx::ZERO]);
            for spot in p.points_in([0, 0, 0], [4096, 1, 4096]) {
                at[(spot.leaf[0].rem_euclid(64) / 16 + spot.leaf[2].rem_euclid(64) / 16 * 4) as usize] += 1.0;
            }
        }
        let mean = at.iter().sum::<f64>() / 16.0;
        assert!(at.iter().all(|n| (n / mean - 1.0).abs() < 0.08), "{at:?}");
    }

    #[test]
    fn a_strip_narrower_than_a_root_holds_points_as_its_chance_has_them_wherever_it_lies() {
        // Three leaves wide, along an axis and a diagonal, slid across the roots: to within a leaf at its edges.
        // Chances read at roots' corners alone, it held none or nearly three times as many.
        let (leaf, root, width, half) = (16, 64, 48.0, 4096);
        for diagonal in [false, true] {
            for from in (0..root).step_by(8) {
                let p = Points::new("strip", 1, &[0, 2], leaf, root, 1, move |[x, _, z]| {
                    let across = if diagonal { (x - z) as f64 / std::f64::consts::SQRT_2 } else { x as f64 };
                    let mut c = [Fx::ZERO; MARKS];
                    c[0] = if (from as f64..from as f64 + width).contains(&across) { fx(0.5) } else { Fx::ZERO };
                    c
                });
                let length = 2.0 * half as f64 * if diagonal { std::f64::consts::SQRT_2 } else { 1.0 };
                let expected = 0.5 * width * length / (leaf * leaf) as f64;
                let held = p.points_in([-half, 0, -half], [half, 1, half]).len() as f64;
                assert!(
                    (held / expected - 1.0).abs() < leaf as f64 / width,
                    "{held} points, expected {expected}, diagonally {diagonal}, from {from}"
                );
            }
        }
    }

    /** Rooms of `points(7)`'s things reaching 100, weighed by where they are, `gaps` apart. */
    fn rooms(gaps: [i64; 2]) -> Rooms {
        Rooms::new(Rc::new(points(7)), 100, 2000, gaps, |s| (hash(9, s.at[0], s.at[2], s.mark as i64) % 2001) as i64)
    }

    #[test]
    fn rooms_never_meet_and_each_place_is_its_owners() {
        let (apart, tiled) = (rooms([4, 12]), rooms([0, 0]));
        let spots = apart.points.points_in([-512, 0, -512], [512, 1, 512]);
        let at = |s: &Spot| s.at.map(|v| v as f64);
        let (apart_rooms, tiled_rooms): (Vec<Room>, Vec<Room>) =
            spots.iter().map(|s| (apart.room(s.at, s.mark), tiled.room(s.at, s.mark))).unzip();
        let reaches = |s: &Spot, q: [i64; 3]| squared(s.at, q) <= 100 * 100;
        let mut random = Rng::new(3);
        for _ in 0..20_000 {
            let q = [random.below(600) as i64 - 300, 0, random.below(600) as i64 - 300];
            let qf = q.map(|v| v as f64);
            // Apart, no place is in two rooms of things reaching it (things at one point share theirs).
            let mut inside: Vec<_> =
                spots.iter().zip(&apart_rooms).filter(|(s, r)| reaches(s, q) && r.margin(qf) >= 0.0).map(|(s, _)| s.at).collect();
            inside.dedup();
            assert!(inside.len() <= 1, "{q:?} is in the rooms of {inside:?}");
            // Tiled, a place reached is in its owner's room, and no other's beyond it.
            let owner = tiled.owner(q);
            for (s, room) in spots.iter().zip(&tiled_rooms).filter(|(s, _)| reaches(s, q)) {
                if owner.is_some_and(|o| o.at == s.at) {
                    assert!(room.margin(qf) >= -1e-9, "{q:?} is {s:?}'s, outside its room");
                } else {
                    assert!(room.margin(qf) <= 1e-9, "{q:?} is {owner:?}'s, inside {s:?}'s room");
                }
            }
            // Owned only by a thing reaching it: a place may be in the room of one that doesn't, and so no one's.
            assert!(owner.is_none_or(|o| reaches(&o, q)), "{q:?} owned by {owner:?}, which doesn't reach it");
        }
        // As far as `reach` says, a segment keeps its margin; a little further, it doesn't.
        for (s, room) in spots.iter().zip(&apart_rooms).take(200) {
            let to = [s.at[0] as f64 + 90.0, 7.0, s.at[2] as f64 - 60.0];
            let t = room.reach(at(s), to, 4.0);
            let along = |t: f64| [0, 1, 2].map(|i| at(s)[i] + t * (to[i] - at(s)[i]));
            assert!(room.margin(at(s)) < 4.0 || room.margin(along(t)) >= 4.0 - 1e-9, "{s:?} short of its margin at {t}");
            assert!(t >= 1.0 || room.margin(along(t + 0.01)) < 4.0, "{s:?} could reach past {t}");
        }
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
