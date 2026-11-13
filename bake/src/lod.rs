//! Native's chunks: no slices. A chunk is
//! a whole cell of 2^level·FCC (cells.rs), seen along one of the 12 faces'
//! axes in relief over the plane through its site, drawn as the voxels of
//! grid max(0, level − shift): at most M = 2^shift of them across, as
//! slices' atom shift. It depends on nothing else, the camera least of all.
//!
//! The camera only picks the chunks a face composes, and by its depth along
//! the face's axis alone, in whole layers. Each level claims the cells whose
//! sites are at most 2Q · 2^L layers ahead of the camera's layer, counted
//! from a layer of its own sites (`Lod::reach`), and a site is drawn at the finest
//! level claiming it: a level draws the cells it claims that are not all
//! within the finer one's claims. A site claimed by level L − 1 is by level
//! L (Q ≥ 3), so each level's claims hold the finer ones'. Where a cell
//! overlaps finer cells, both draw what is there, and the finer is painted
//! in front.
//!
//! Finer levels in front, and within a level whichever voxel is nearer
//! (the compositor's depth), chunks need no clipping where they meet, for
//! any ray of the face from anywhere: no ray meets a finer level after a
//! coarser one. A face draws the rays nearer its axis F than any other of
//! the 12 directions, and a ray crossing from a cell into its neighbour
//! across n (one of the 12, scaled) has v · n > 0; were n · F < 0, n would
//! be m − F or −F for one of them, m, and v · n = v · m − v · F ≤ 0. So
//! along a ray of the face, a level's cells come layer after layer, and once
//! it leaves a level's claims it is never back.
//!
//! A level's medium (bake's Medium) it draws between two planes across the
//! axis instead, the finer level's reach and its own, from a layer of cells
//! more each way than it claims (Lod::layers).

use fcc::GRID_LEVELS;

use crate::slicing::LAYER;

/** Layers ahead of the camera's layer that the nearest voxels drawn are in: nearer, one would fill the screen. */
const NEAR: i32 = 4;

/** A chunk: the cell of 2^level·FCC around `site`, drawn as the voxels of grid `atoms`, in whose units, in a face's lattice coordinates, the site is. */
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellChunk {
    pub level: i32,
    pub atoms: i32,
    pub site: [i32; 3],
}

impl CellChunk {
    /** M: the spacing of its level's sites in its grid's units, its cell 2M layers deep. */
    pub fn spacing(&self) -> i32 {
        1 << (self.level - self.atoms)
    }

    /** The sites it may hold, a box of (a, b, c) in its grid's units, inclusive: its cell's. */
    pub fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        let m = self.spacing();
        (self.site.map(|v| v - m), self.site.map(|v| v + m))
    }
}

/** How the cells' levels follow depth: grids `shift` levels finer than their cells, level L claiming cells up to 2Q · 2^L layers ahead. */
pub struct Lod {
    pub shift: i32,
    pub q: i32,
}

impl Lod {
    /**
     * For `detail` (viewing distance over thickness, as slices have it):
     * a cell 2N layers deep is drawn from Q · N layers ahead on, Q at
     * least 2 · detail; and at least 4, so that no cell but single voxels
     * reaches the near plane.
     */
    pub fn new(shift: i32, detail: f64) -> Lod {
        Lod { shift, q: ((2.0 * detail).ceil() as i32).max(4) }
    }

    /** The coarsest level: cells of the coarsest grid. */
    pub fn top(&self) -> i32 {
        GRID_LEVELS as i32 - 1 + self.shift
    }

    /**
     * The last layer whose cells `level` claims, for a camera in layer
     * `camera`: 2Q · N layers past the last of the level's layers of
     * sites not past the camera's; none for the coarsest, which claims all.
     */
    fn reach(&self, level: i32, camera: i32) -> i64 {
        let n = 1i64 << level;
        if level == self.top() { i64::MAX } else { (camera as i64).div_euclid(n) * n + 2 * self.q as i64 * n }
    }

    /**
     * How far from the eye, in grid `grid`'s sites, the chunks drawn of it
     * to `far` (world units) ahead read it, whichever way the eye turns; none
     * if no level draws from it. A ray of a face meets each cell drawn, and
     * all the chunk reads (chunk_reads) is ahead of the eye no deeper than
     * its far side, and off the ray no more than its width across. A point
     * of a ray, or ahead of one, is at most √2 times as far as it is ahead:
     * as many sites as layers (LAYER).
     */
    pub fn grid_reach(&self, grid: usize, far: f64) -> Option<f64> {
        // An eye in the layer before a site of every level, or there: where claims begin nearest, and where they go farthest.
        let eyes = [[0.0; 2], [0.0; 2], [-1.0, 0.0]];
        (0..=self.top())
            .filter(|&level| (level - self.shift).max(0) as usize == grid)
            .filter_map(|level| {
                let k = *self.layers(level, &eyes, far)?.end() as i32;
                let (_, min, max) = crate::chunk_reads(&cell(self.shift, level, [k & 1, k & 1, k]));
                Some(f64::from(max[2]) + (f64::from(max[0] - min[0]) * LAYER).hypot(f64::from(max[1] - min[1])))
            })
            .reduce(f64::max)
    }

    /**
     * The layers of `level`'s sites, c / N, it claims for an eye in `eyes`
     * (Eyes) and to `far` ahead, and a layer more each way, whose cells'
     * medium it draws: from the nearest eye's claims to the farthest's, as
     * reach grows with the camera's layer; none, nor any coarser level's, if
     * its first is past the farthest eye's far.
     * It draws medium past the finer level's reach to its own, by slabs of
     * columns (bake's Medium), each the cell's holding its middle: up to N
     * layers and half a slab from a voxel drawn, a layer of cells off.
     */
    fn layers(&self, level: i32, eyes: &Eyes, far: f64) -> Option<std::ops::RangeInclusive<i64>> {
        let n = 1i64 << level;
        // Any whole layer near the eye would do: the levels' order holds for any.
        let (low, high) = (eyes[2][0].floor() as i32, eyes[2][1].floor() as i32);
        let last = high as i64 + (far / LAYER).ceil() as i64;
        // Claimed, and not all within the finer claims: its sites reach N layers past its own; and a layer more each way, for its medium.
        let first = if level == 0 { low as i64 + NEAR as i64 } else { self.reach(level - 1, low) - 2 * n + 1 };
        let to = self.reach(level, high).saturating_add(n).min(last + n);
        (first <= last).then(|| first.div_euclid(n) + i64::from(first.rem_euclid(n) != 0)..=to.div_euclid(n))
    }

    /**
     * Of `level`'s layer of sites c = k · N, those whose cells may meet a
     * ray within `tangents` from an eye in `eyes`: [a0, a1, b0, b1], in
     * N's; none if all are behind every eye.
     */
    fn rows(level: i32, k: i64, eyes: &Eyes, tangents: [f64; 4]) -> Option<[i32; 4]> {
        let nf = (1i64 << level) as f64;
        let c = k as f64 * nf;
        // The cell's depths ahead of the eye, along the axis, nearest and farthest.
        let (z0, z1) = (((c - nf - eyes[2][1]) * LAYER).max(0.0), (c + nf - eyes[2][0]) * LAYER);
        if z1 <= 0.0 {
            return None;
        }
        let span = |lo: f64, hi: f64| (lo.min(lo * z0 / z1) * z1, hi.max(hi * z0 / z1) * z1);
        let (x0, x1) = span(tangents[0], tangents[1]);
        let (y0, y1) = span(tangents[2], tangents[3]);
        let [[a_lo, a_hi], [b_lo, b_hi], _] = *eyes;
        Some([
            ((x0 / LAYER + a_lo) / nf - 1.0).floor() as i32,
            ((x1 / LAYER + a_hi) / nf + 1.0).ceil() as i32,
            ((y0 + b_lo) / nf - 1.0).floor() as i32,
            ((y1 + b_hi) / nf + 1.0).ceil() as i32,
        ])
    }

    /**
     * The cells a face may compose (`composed`) from an eye anywhere in
     * `eyes`, its rays within `tangents`, kept by layer and row, each with a
     * `T` of the caller's: taken from the `old` pick where it had the cell,
     * at the same level of detail, so that moving a little keeps what is
     * known of most; what it does not take stays there.
     */
    pub fn pick<T: Default>(&self, eyes: Eyes, far: f64, tangents: [f64; 4], old: Option<&mut Picked<T>>) -> Picked<T> {
        let lod = (self.shift, self.q, far);
        let mut old = old.filter(|o| o.lod == lod);
        let mut levels = Vec::new();
        for level in 0..=self.top() {
            let Some(ks) = self.layers(level, &eyes, far) else { break };
            let first = *ks.start();
            let layers = ks
                .map(|k| {
                    let Some([a0, a1, b0, b1]) = Lod::rows(level, k, &eyes, tangents) else { return Layer::default() };
                    // Sites of N·FCC: a / N, b / N, c / N of one parity.
                    let parity = |v: i32| v + ((v ^ k as i32) & 1);
                    let (a, b) = (parity(a0), parity(b0));
                    let (across, rows) = (((a1 - a) / 2 + 1).max(0) as usize, ((b1 - b) / 2 + 1).max(0) as usize);
                    let mut was = old.as_mut().and_then(|o| o.layer(level, k));
                    let mut layer = Layer { a, b, across, rows, cells: Vec::new() };
                    layer.cells = (0..rows * across)
                        .map(|at| {
                            let [i, j] = layer.site(at);
                            was.as_mut().and_then(|w| w.at(i, j)).map(std::mem::take).unwrap_or_default()
                        })
                        .collect();
                    layer
                })
                .collect();
            levels.push((first, layers));
        }
        Picked { lod, eyes, tangents, levels }
    }

    /**
     * Whether a face composes `cell` from `eye`, its rays within
     * `tangents`, to `far` ahead (world units): whether `composed` from
     * that eye alone visits it, at once.
     */
    pub fn composes(&self, cell: &CellChunk, eye: [f64; 3], far: f64, tangents: [f64; 4]) -> bool {
        let eyes = eye.map(|v| [v, v]);
        let level = cell.level;
        // Its site (i, j, k) in N's.
        let [i, j, k] = cell.site.map(|v| (v << cell.atoms) >> level);
        let in_rows = |rows: Option<[i32; 4]>, d: usize, v: i32| rows.is_some_and(|r| (r[2 * d]..=r[2 * d + 1]).contains(&v));
        self.layers(level, &eyes, far).is_some_and(|ks| ks.contains(&i64::from(k)))
            && in_rows(Lod::rows(level, k.into(), &eyes, tangents), 1, j)
            && in_rows(in_face(*cell, &eyes, tangents).and_then(|t| Lod::rows(level, k.into(), &eyes, t)), 0, i)
    }

    /**
     * The chunks a face composes from an eye anywhere in `eyes` (Eyes): the
     * cells each level draws for one of them, from its layer to the pick's
     * `far` (world units) ahead along the axis, whose rays have x / z and
     * y / z within `tangents` (min x, max x, min y, max y; the face's
     * coordinates, see view.ts); each with its `T` in `picked`, which covers
     * them (Picked::covers).
     */
    pub fn composed<T>(&self, picked: &mut Picked<T>, eyes: Eyes, tangents: [f64; 4], mut visit: impl FnMut(CellChunk, &mut T)) {
        for level in 0..=self.top() {
            let Some(ks) = self.layers(level, &eyes, picked.lod.2) else { break };
            for k in ks {
                let Some([_, _, b0, b1]) = Lod::rows(level, k, &eyes, tangents) else { continue };
                let layer = picked.layer(level, k).expect("covered");
                for r in within(layer.b, layer.rows, b0, b1) {
                    let j = layer.b + 2 * r as i32;
                    let row = in_face(cell(self.shift, level, [layer.a, j, k as i32]), &eyes, tangents);
                    let Some([a0, a1, _, _]) = row.and_then(|t| Lod::rows(level, k, &eyes, t)) else { continue };
                    for s in within(layer.a, layer.across, a0, a1) {
                        let i = layer.a + 2 * s as i32;
                        visit(cell(self.shift, level, [i, j, k as i32]), &mut layer.cells[r * layer.across + s]);
                    }
                }
            }
        }
    }
}

/**
 * Of `tangents`, those the face's rays (|x| + √2 |y| ≤ z) may have
 * through the voxels of the cells in `row`'s row (its j and k) from an eye
 * in `eyes`: x / z within ±(1 − √2 |y| / z) for their least |y| / z (their
 * sites within N of the cells', the voxels a voxel past); none if no ray
 * of the face meets them. A rectangle of tangents (Lod::rows) holds the
 * face's rhombus, and its corners, cells none of its rays meets.
 */
fn in_face(row: CellChunk, eyes: &Eyes, [x0, x1, y0, y1]: [f64; 4]) -> Option<[f64; 4]> {
    let (g, n) = ((1i64 << row.atoms) as f64, (1i64 << row.level) as f64);
    let [_, j, k] = row.site.map(|v| f64::from(v) * g);
    let far = (k + n + g - eyes[2][0]) * LAYER;
    let (low, high) = (j - n - g - eyes[1][1], j + n + g - eyes[1][0]);
    let least = if low > 0.0 { low } else { (-high).max(0.0) };
    let w = 1.0 - std::f64::consts::SQRT_2 * least / far;
    let (x0, x1) = (x0.max(-w), x1.min(w));
    (far > 0.0 && x0 <= x1).then_some([x0, x1, y0, y1])
}

/** The cell of `level` whose site is (i, j, k) · N, its voxels `shift` levels finer (Lod's). */
fn cell(shift: i32, level: i32, site: [i32; 3]) -> CellChunk {
    let atoms = (level - shift).max(0);
    CellChunk { level, atoms, site: site.map(|v| (v << level) >> atoms) }
}

/** Of `len` sites from `first`, 2 apart, those in [lo, hi], by index. */
fn within(first: i32, len: usize, lo: i32, hi: i32) -> std::ops::Range<usize> {
    let index = |v: i32| v.clamp(0, len as i32) as usize;
    index((lo - first + 1).div_euclid(2))..index((hi - first).div_euclid(2) + 1)
}

/** A layer of a level's sites picked: from (a, b), in N's, `rows` rows of `across` sites, both 2 apart, and theirs. */
#[derive(Default)]
struct Layer<T> {
    a: i32,
    b: i32,
    across: usize,
    rows: usize,
    cells: Vec<T>,
}

impl<T> Layer<T> {
    /** The site (i, j) of its `at`-th cell. */
    fn site(&self, at: usize) -> [i32; 2] {
        [self.a + 2 * (at % self.across) as i32, self.b + 2 * (at / self.across) as i32]
    }

    /** The site's (i, j), of the layer's parity, if it has it. */
    fn at(&mut self, i: i32, j: i32) -> Option<&mut T> {
        let (s, r) = (usize::try_from(i - self.a).ok()? / 2, usize::try_from(j - self.b).ok()? / 2);
        (s < self.across && r < self.rows).then(|| &mut self.cells[r * self.across + s])
    }
}

impl<T> Picked<T> {
    /** Whether it holds all a face composes from `eye`, its rays within `tangents`, at `lod` (Picked's). */
    pub fn covers(&self, lod: (i32, i32, f64), eye: [f64; 3], tangents: [f64; 4]) -> bool {
        let [w0, w1, w2, w3] = self.tangents;
        self.lod == lod
            && (0..3).all(|d| (self.eyes[d][0]..=self.eyes[d][1]).contains(&eye[d]))
            && tangents[0] >= w0
            && tangents[1] <= w1
            && tangents[2] >= w2
            && tangents[3] <= w3
    }

    /** The eyes and tangents it was picked for: those it composes all of. */
    pub fn bounds(&self) -> (Eyes, [f64; 4]) {
        (self.eyes, self.tangents)
    }

    /** The `T` of `cell`, if it has it. */
    pub fn at(&mut self, cell: &CellChunk) -> Option<&mut T> {
        let [i, j, k] = cell.site.map(|v| (v << cell.atoms) >> cell.level);
        self.layer(cell.level, k.into())?.at(i, j)
    }

    /** Each of its cells, with its `T`. */
    pub fn into_each(self, mut visit: impl FnMut(CellChunk, T)) {
        for (level, (first, layers)) in (0..).zip(self.levels) {
            for (k, mut layer) in (first..).zip(layers) {
                for (at, t) in std::mem::take(&mut layer.cells).into_iter().enumerate() {
                    let [i, j] = layer.site(at);
                    visit(cell(self.lod.0, level, [i, j, k as i32]), t);
                }
            }
        }
    }

    /** Layer k = c / N of `level`, if it has it. */
    fn layer(&mut self, level: i32, k: i64) -> Option<&mut Layer<T>> {
        let (first, layers) = self.levels.get_mut(usize::try_from(level).ok()?)?;
        layers.get_mut(usize::try_from(k - *first).ok()?)
    }
}

/**
 * A face's cells picked for an eye anywhere in `eyes` and rays within
 * `tangents` (Lod::pick), at a level of detail (`lod`: Lod's shift and Q,
 * and how far ahead): per level, its first layer, k = c / N, and its
 * layers, each with a `T` per cell.
 */
pub struct Picked<T> {
    lod: (i32, i32, f64),
    eyes: Eyes,
    tangents: [f64; 4],
    levels: Vec<(i64, Vec<Layer<T>>)>,
}

/** The eye anywhere in a box: [low, high] of each of its lattice coordinates, in units of grid 0. */
pub type Eyes = [[f64; 2]; 3];

/** How far a cell's site is from `eye` (by which the nearest are baked first), both in a face's lattice coordinates, in units of grid 0. */
pub fn distance(cell: &CellChunk, eye: [f64; 3]) -> f64 {
    let [a, b, c] = std::array::from_fn(|d| (cell.site[d] << cell.atoms) as f64 - eye[d]);
    ((a * a + c * c) / 2.0 + b * b).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::owner;

    use std::collections::HashSet;

    /** The cells a face composes from `eye`, its rays within `tangents`, of `picked`: each once. */
    fn composed_of<T>(lod: &Lod, picked: &mut Picked<T>, eye: [f64; 3], tangents: [f64; 4]) -> HashSet<CellChunk> {
        let mut cells = HashSet::new();
        lod.composed(picked, eye.map(|v| [v, v]), tangents, |cell, _| assert!(cells.insert(cell), "{cell:?} twice"));
        cells
    }

    /** The cells a face composes from `eye`, its rays within `tangents`, picked there. */
    fn composed_from(lod: &Lod, eye: [f64; 3], far: f64, tangents: [f64; 4]) -> HashSet<CellChunk> {
        composed_of(lod, &mut lod.pick::<()>(eye.map(|v| [v, v]), far, tangents, None), eye, tangents)
    }

    /** The chunks a face composes from `eye`, by level and site in units of grid 0. */
    fn composed(lod: &Lod, eye: [f64; 3], far: f64) -> HashSet<(i32, [i32; 3])> {
        let rhombus = [-1.0, 1.0, -std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];
        composed_from(lod, eye, far, rhombus).into_iter().map(|c| (c.level, c.site.map(|v| v << c.atoms))).collect()
    }

    #[test]
    fn a_pick_composes_from_an_eye_in_its_box_what_one_made_there_would_and_keeps_what_it_knew() {
        let (lod, far) = (Lod { shift: 3, q: 19 }, 300.0);
        let (wide, tangents) = ([-0.6, 0.5, -0.3, 0.4], [-0.5, 0.45, -0.25, 0.3]);
        let around = |centre: [f64; 3]| centre.map(|v| [v - 2.0, v + 2.0]);
        let centre = [3.3, -1.2, 7.8];
        let mut old: Picked<Option<CellChunk>> = lod.pick(around(centre), far, wide, None);
        let mut known = HashSet::new();
        for d in [[-2.0, 2.0, -2.0], [1.9, -0.4, 1.99], [0.0, 0.0, 0.0], [-1.1, -2.0, 0.6]] {
            let eye = std::array::from_fn(|i| centre[i] + d[i]);
            let there = composed_from(&lod, eye, far, tangents);
            assert!(there.len() > 1000, "{}", there.len());
            assert_eq!(composed_of(&lod, &mut old, eye, tangents), there, "eye {eye:?}");
            lod.composed(&mut old, eye.map(|v| [v, v]), tangents, |cell, seen| *seen = Some(cell));
            known.extend(there);
        }
        // Picked again a little farther on: what it knew of a cell is the new pick's, where `at` finds it, or stays in the old.
        let moved = centre.map(|v| v + 2.5);
        assert!(!old.covers((lod.shift, lod.q, far), moved, tangents));
        let mut new = lod.pick(around(moved), far, wide, Some(&mut old));
        assert!(known.iter().all(|cell| new.at(cell).is_none_or(|seen| *seen == Some(*cell))));
        let mut kept = HashSet::new();
        new.into_each(|cell, seen| assert!(seen.is_none_or(|s| s == cell && kept.insert(cell)), "{cell:?} has {seen:?}"));
        assert!(kept.len() * 2 > known.len(), "{} of {} kept", kept.len(), known.len());
        old.into_each(|cell, seen| assert!(seen.is_none_or(|s| s == cell && kept.insert(cell)), "{cell:?} has {seen:?}"));
        assert_eq!(kept, known);
    }

    #[test]
    fn a_pick_composes_for_its_whole_box_all_that_an_eye_in_it_composes_cell_by_cell() {
        let (lod, far) = (Lod { shift: 3, q: 8 }, 300.0);
        let (wide, tangents) = ([-0.8, 0.7, -0.5, 0.6], [-0.65, 0.55, -0.35, 0.45]);
        let centre = [-4.1, 2.3, 11.6];
        let mut picked: Picked<()> = lod.pick(centre.map(|v| [v - 3.0, v + 3.0]), far, wide, None);
        let (eyes, bounds) = picked.bounds();
        let mut all = HashSet::new();
        lod.composed(&mut picked, eyes, bounds, |cell, _| assert!(all.insert(cell), "{cell:?} twice"));
        for d in [[-3.0, 3.0, -3.0], [2.9, -1.4, 2.99], [0.0, 0.0, 0.0], [-1.7, -3.0, 0.4]] {
            let eye = std::array::from_fn(|i| centre[i] + d[i]);
            let there = composed_of(&lod, &mut picked, eye, tangents);
            let composes: HashSet<CellChunk> = all.iter().copied().filter(|cell| lod.composes(cell, eye, far, tangents)).collect();
            assert_eq!(composes, there, "eye {eye:?}: composes, cell by cell, as composed");
            assert!(
                there.len() > 1000 && there.is_subset(&all),
                "eye {eye:?}: {} of {} not in the box's",
                there.difference(&all).count(),
                there.len()
            );
        }
    }

    /** Whether a point (a, b, c), in a face's lattice coordinates, is in its rays and within `far` of `eye`, well within. */
    fn in_view(eye: [f64; 3], p: [f64; 3], far: f64) -> bool {
        let (x, y, z) = ((p[0] - eye[0]) * LAYER, p[1] - eye[1], (p[2] - eye[2]) * LAYER);
        x.abs() + std::f64::consts::SQRT_2 * y.abs() <= z - 0.01 && (x * x + y * y + z * z).sqrt() <= far
    }

    #[test]
    fn every_site_ahead_is_drawn_first_at_the_finest_level_claiming_it_and_its_medium_by_a_cell_composed() {
        let far = 150.0;
        for (k, shift) in [(0, 1), (1, 2), (2, 3)] {
            let lod = Lod { shift, q: 4 + k };
            let eye = [0.3 + 7.1 * k as f64, -2.6 + k as f64, 1.7 - 3.3 * k as f64];
            let camera = eye[2].floor() as i32;
            let near = camera + NEAR;
            let chunks = composed(&lod, eye, far);
            let levels: std::collections::BTreeSet<i32> = chunks.iter().map(|k| k.0).collect();
            assert!(levels.len() >= 3, "levels {levels:?}");
            // Where a level's medium is, past the finer level's reach to its own.
            let medium = |l: i32, c: i64| (l == 0 || c > lod.reach(l - 1, camera)) && c <= lod.reach(l, camera);
            let mut ahead = 0;
            for a in -90..=90 {
                for b in -24..=24 {
                    for c in -10..=150 {
                        let p = [a, b, c];
                        if (a ^ c) & 1 != 0 || (b ^ c) & 1 != 0 || !in_view(eye, p.map(f64::from), far) {
                            continue;
                        }
                        // The chunks holding it whose whole voxels are drawn, finest first: claimed, not those a layer more each way, whose medium alone is.
                        let claims = |l: i32| {
                            let c = i64::from(owner(1 << l, p)[2]);
                            (l == 0 || c > lod.reach(l - 1, camera) - (1 << l)) && c <= lod.reach(l, camera)
                        };
                        let holding: Vec<i32> =
                            levels.iter().copied().filter(|&l| claims(l) && chunks.contains(&(l, owner(1 << l, p)))).collect();
                        if c < near {
                            assert!(holding.is_empty(), "shift {shift}: {p:?}, before the near plane, in {holding:?}");
                            continue;
                        }
                        let finest = (0..).find(|&l| owner(1 << l, p)[2] as i64 <= lod.reach(l, camera));
                        assert_eq!(holding.first().copied(), finest, "shift {shift} site {p:?}");
                        // Where it is a site of a level's grid within its medium, its slab of its column is a cell's composed (bake's Medium).
                        for &l in &levels {
                            let atoms = (l - shift).max(0);
                            let s = 1 << atoms;
                            if owner(s, p) != p || !medium(l, i64::from(c)) {
                                continue;
                            }
                            let [a, b, c] = p.map(|v| v / s);
                            let middle = crate::splats::slab_middle(c, b, crate::slab_layers(1 << (l - atoms)));
                            let held = (l, owner(1 << l, [a * s, b * s, middle * s]));
                            assert!(chunks.contains(&held), "shift {shift} site {p:?}: its slab of level {l} in {held:?}");
                        }
                        ahead += 1;
                    }
                }
            }
            assert!(ahead > 10000, "{ahead}");
        }
    }

    /**
     * The levels a ray from `eye` meets, where each shows (the finest of
     * the chunks holding a point): how many times it goes from one to
     * another, and how many of those to a finer one.
     */
    fn crossings(lod: &Lod, eye: [f64; 3], far: f64) -> (usize, usize) {
        let chunks = composed(lod, eye, far);
        let shows = |x: [f64; 3]| (0..=lod.top()).find(|&l| chunks.contains(&(l, nearest(1 << l, x))));
        let (mut met, mut wrong) = (0, 0);
        for r in 0..200 {
            let (u, v) = ((r * 37 % 101) as f64 / 101.0 * 1.9 - 0.95, (r * 53 % 97) as f64 / 97.0 * 1.3 - 0.65);
            let dir = [u / LAYER, v * (1.0 - u.abs()) / std::f64::consts::SQRT_2, 1.0 / LAYER];
            let mut last: Option<i32> = None;
            for t in 6..1600 {
                let x: [f64; 3] = std::array::from_fn(|d| eye[d] + dir[d] * t as f64 * 0.07);
                if !in_view(eye, x, far) {
                    continue;
                }
                let Some(now) = shows(x) else { continue };
                if let Some(before) = last
                    && before != now
                {
                    met += 1;
                    wrong += usize::from(now < before);
                }
                last = Some(now);
            }
        }
        (met, wrong)
    }

    #[test]
    fn no_ray_meets_a_finer_level_after_a_coarser_from_anywhere() {
        let (mut met, mut wrong) = (0, 0);
        for e in 0..6 {
            let lod = Lod { shift: 2, q: 4 + e % 3 };
            let eye = [0.37 * e as f64 - 2.0, 0.21 * (e % 5) as f64 - 0.4, 0.29 * (e % 7) as f64 - 30.0 * e as f64];
            let (m, w) = crossings(&lod, eye, 120.0);
            (met, wrong) = (met + m, wrong + w);
        }
        assert!(met > 1000 && wrong == 0, "{wrong} of {met} crossings to a finer level");
    }

    #[test]
    fn the_chunks_of_each_grid_read_it_within_its_reach_and_nearly_as_far() {
        let rhombus = [-1.0, 1.0, -std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];
        // Some reaching every level as far as Q allows, the coarsest to `far`; some short of the coarser levels.
        for (shift, q, far) in [(3, 20, 40000.0), (3, 4, 10000.0), (2, 7, 10000.0), (3, 20, 8000.0), (3, 20, 2000.0), (1, 5, 300.0)] {
            let lod = Lod { shift, q };
            let mut most = [0.0f64; GRID_LEVELS];
            for e in 0..6 {
                let eye = [0.37 * e as f64 - 2.0, 0.21 * (e % 5) as f64 - 0.4, 3.29 * e as f64 - 1.0];
                let mut picked = lod.pick::<()>(eye.map(|v| [v, v]), far, rhombus, None);
                lod.composed(&mut picked, eye.map(|v| [v, v]), rhombus, |cell, _| {
                    let (g, min, max) = crate::chunk_reads(&cell);
                    let s = (1i64 << g) as f64;
                    for corner in 0..8 {
                        let [a, b, c]: [f64; 3] =
                            std::array::from_fn(|d| if corner >> d & 1 == 0 { min[d] } else { max[d] } as f64 * s - eye[d]);
                        most[g] = most[g].max(((a * a + c * c) / 2.0 + b * b).sqrt() / s);
                    }
                });
            }
            for (g, most) in most.into_iter().enumerate() {
                let reach = lod.grid_reach(g, far);
                // Short of it by less than the reads' width across, which it adds whole off the ray.
                let (_, min, max) = crate::chunk_reads(&cell(shift, g as i32 + shift, [0; 3]));
                let across = (f64::from(max[0] - min[0]) * LAYER).hypot(f64::from(max[1] - min[1]));
                let near = reach.is_some_and(|r| most <= r && most > r - across);
                assert!(
                    near || reach.is_none() && most == 0.0,
                    "shift {shift}, Q {q}, far {far}: grid {g} read {most:.0} sites far, its reach {reach:?}"
                );
            }
        }
    }

    /** The site of N·FCC nearest point x, in a face's lattice coordinates. */
    fn nearest(n: i32, x: [f64; 3]) -> [i32; 3] {
        let mut best = ([0; 3], f64::INFINITY);
        let base = x.map(|v| (v / n as f64).floor() as i32);
        for da in -1..=2 {
            for db in -1..=2 {
                for dc in -1..=2 {
                    let (i, j, k) = (base[0] + da, base[1] + db, base[2] + dc);
                    if (i ^ k) & 1 != 0 || (j ^ k) & 1 != 0 {
                        continue;
                    }
                    let s = [i * n, j * n, k * n];
                    let (a, b, c) = (s[0] as f64 - x[0], s[1] as f64 - x[1], s[2] as f64 - x[2]);
                    let d = (a * a + c * c) / 2.0 + b * b;
                    if d < best.1 {
                        best = (s, d);
                    }
                }
            }
        }
        best.0
    }
}
