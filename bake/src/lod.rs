//! Native's chunks. A chunk is a cell of the octree, the cube of 2^level
//! sites of grid 0 a side (lattice's levels), seen along one of the 18
//! faces' axes (face.rs), drawn as the voxels of grid max(0, level −
//! shift): at most M = 2^shift of them across. It depends on nothing else,
//! the camera least of all.
//!
//! In a face's lattice coordinates a cell of level L, the cube of sites
//! N q + [0, N)³ (N = 2^L), is at (I, J, K) = (q · R, q · U, q · F), I ≡ K
//! (mod k), as a site is: K is its layer of cells. The camera only picks
//! the chunks a face composes, by their layer alone. Each level claims the
//! cells of layers K with K · N at most 2Q · N layers of grid 0 ahead of
//! the camera's (`Lod::reach`), and a site is drawn at the finest level
//! claiming its cell: a level draws the cells it claims of which some
//! child is past the finer level's claims (`Lod::layers`). Where a cell
//! overlaps finer cells, both draw what is there, and the finer is painted
//! in front.
//!
//! Finer levels in front, and within a level whichever is nearer (the
//! compositor's depth), chunks need no clipping where they meet, for any
//! ray of the face from anywhere: no ray meets a finer level after a
//! coarser one. A face draws the rays nearer its axis F than any other of
//! the 18 directions, which go along each world axis F goes along as F
//! does: so a ray going from a cell into its neighbour across a face goes
//! to a layer K no less, and once it leaves a level's claims, which only
//! shrink as K grows, it is never back.
//!
//! A cell's medium (bake's Medium) is a chunk of its own, picked by a Lod
//! of its own (`Lod::for_medium`): the same grid as far away as whole
//! voxels, in cells MEDIUM_COARSER levels coarser, so that a ray crosses
//! that many times fewer slabs, a fragment of each pixel each. A level
//! draws it between two planes across the axis, the finer level's reach
//! and its own, from a layer of cells more each way than it claims
//! (Lod::layers); none finer than MEDIUM_LEVEL, which draws it from the
//! near plane on: finer cells would be thousands of chunks round the eye.

use lattice::GRID_LEVELS;

use crate::face::Face;

/** Layers ahead of the camera's layer that the nearest voxels drawn are in: nearer, one would fill the screen. */
pub const NEAR: i32 = 4;

/** Levels the medium's cells are coarser than those of whole voxels of the same grid (Lod::for_medium). */
const MEDIUM_COARSER: i32 = 2;

/** The finest level whose cells draw medium, cells 32 voxels across, if they are grid 0's (Lod::finest). */
const MEDIUM_LEVEL: i32 = 5;

/** The sum of a vector's positive components, and of its negative ones: how far its dot product with [0, 1]³ reaches each way. */
pub fn spread(v: [i32; 3]) -> [i32; 2] {
    [v.iter().map(|&c| c.min(0)).sum(), v.iter().map(|&c| c.max(0)).sum()]
}

/**
 * A chunk: the cell `cell` of `level` (its sites of that level's grid),
 * drawn as the voxels of grid `atoms`; its whole voxels, or its medium.
 */
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellChunk {
    pub level: i32,
    pub atoms: i32,
    pub cell: [i32; 3],
    pub medium: bool,
}

impl CellChunk {
    /** M: how many sites of its grid it is across. */
    pub fn spacing(&self) -> i32 {
        1 << (self.level - self.atoms)
    }

    /** The sites of its grid it holds, a box, inclusive. */
    pub fn sites(&self) -> ([i32; 3], [i32; 3]) {
        let m = self.spacing();
        (self.cell.map(|v| v * m), self.cell.map(|v| v * m + m - 1))
    }

    /** Whether it holds site `p` of its grid. */
    pub fn holds(&self, p: [i32; 3]) -> bool {
        let m = self.spacing();
        (0..3).all(|d| p[d].div_euclid(m) == self.cell[d])
    }

    /** (I, J, K) along `face`. */
    pub fn index(&self, face: &Face) -> [i32; 3] {
        face.of(self.cell)
    }

    /** Twice its middle's c along `face`, in its grid's lattice coordinates: the plane its chunk is drawn over. */
    pub fn centre(&self, face: &Face) -> i32 {
        let m = self.spacing();
        2 * face.of(self.cell)[2] * m + (m - 1) * (face.f[0] + face.f[1] + face.f[2])
    }

    /** The least lattice coordinates (a, b) of its sites along `face`: its column (0, 0). */
    pub fn columns(&self, face: &Face) -> [i32; 2] {
        let (m, [i, j, _]) = (self.spacing(), face.of(self.cell));
        [i * m + spread(face.r)[0] * (m - 1), j * m]
    }

    /** The least and the greatest c of its sites along `face`, in its grid's lattice coordinates. */
    pub fn layer_range(&self, face: &Face) -> [i32; 2] {
        let (m, k) = (self.spacing(), face.of(self.cell)[2]);
        spread(face.f).map(|s| k * m + s * (m - 1))
    }
}

/**
 * How the cells' levels follow depth, the cells of whole voxels or of
 * `medium`: grids `shift` levels finer than their cells, level L claiming
 * cells up to 2Q · 2^L layers ahead.
 */
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lod {
    pub shift: i32,
    pub q: i32,
    pub medium: bool,
}

impl Lod {
    /**
     * For `detail` (viewing distance over thickness):
     * a cell N layers deep is drawn from 2Q · N layers ahead on, Q at
     * least detail; and at least 4, so that no cell but single voxels
     * reaches the near plane.
     */
    pub fn new(shift: i32, detail: f64) -> Lod {
        Lod { shift, q: (detail.ceil() as i32).max(4), medium: false }
    }

    /**
     * The medium's, of these whole voxels': its cells MEDIUM_COARSER levels
     * coarser, each grid drawn as far (2^MEDIUM_COARSER times fewer cells
     * deep a level, Q ≥ 3 so that each level's claims hold the finer ones').
     */
    pub fn for_medium(&self) -> Lod {
        let q = ((self.q + (1 << MEDIUM_COARSER) - 1) >> MEDIUM_COARSER).max(3);
        Lod { shift: self.shift + MEDIUM_COARSER, q, medium: true }
    }

    /** The finest level it draws: 0, or of medium, MEDIUM_LEVEL, or finer, so that it draws grid 0's voxels. */
    pub fn finest(&self) -> i32 {
        if self.medium { MEDIUM_LEVEL.min(self.shift) } else { 0 }
    }

    /** The coarsest level: cells of the coarsest grid. */
    pub fn top(&self) -> i32 {
        GRID_LEVELS as i32 - 1 + self.shift
    }

    /** The grid `level` draws its cells as the voxels of. */
    pub fn grid(&self, level: i32) -> i32 {
        (level - self.shift).max(0)
    }

    /** The cell of `level` at (I, J, K) along `face`, its voxels `shift` levels finer: its whole voxels, or its medium. */
    pub fn cell(&self, face: &Face, level: i32, index: [i32; 3]) -> CellChunk {
        CellChunk { level, atoms: self.grid(level), cell: face.world(index), medium: self.medium }
    }

    /**
     * The last layer of grid 0 whose cells `level` claims, for a camera in
     * layer `camera`: 2Q · N layers past the last multiple of N not past
     * the camera's; none for the coarsest, which claims all.
     */
    pub fn reach(&self, level: i32, camera: i32) -> i64 {
        let n = 1i64 << level;
        if level == self.top() { i64::MAX } else { (camera as i64).div_euclid(n) * n + 2 * self.q as i64 * n }
    }

    /**
     * The layers K of `level`'s cells along `face` whose whole voxels it
     * claims for an eye in `eyes` and to `far` ahead, or (`medium`) whose
     * medium it draws, a layer more each way: from the nearest eye's
     * claims to the farthest's, as reach grows with the camera's layer;
     * none if its first is past the farthest eye's far, nor finer than its
     * finest. A cell is drawn if some child of it is past the finer
     * level's reach (or, the finest, past the near plane): its farthest
     * child, 2K + max(F, 0) of the finer level, half its layers past
     * (`drawn`).
     */
    fn layers(&self, face: &Face, level: i32, eyes: &Eyes, far: f64) -> Option<std::ops::RangeInclusive<i64>> {
        if level < self.finest() {
            return None;
        }
        let n = 1i64 << level;
        let (low, high) = (eyes[2][0].floor() as i32, eyes[2][1].floor() as i32);
        let last = high as i64 + (far / face.layer()).ceil() as i64;
        let more = if self.medium { n } else { 0 };
        let from = self.from(level, low) - more;
        let to = self.reach(level, high).saturating_add(more).min(last + n);
        // The least K with 2 K N > 2 from − max(F, 0) N.
        let first = (2 * from - spread(face.f)[1] as i64 * n).div_euclid(2 * n) + 1;
        (first * n <= last).then(|| first..=to.div_euclid(n))
    }

    /** The layer of grid 0 past which `level` draws: the finer level's reach, or the near plane, the finest. */
    fn from(&self, level: i32, camera: i32) -> i64 {
        if level == self.finest() { (camera + NEAR) as i64 } else { self.reach(level - 1, camera) }
    }

    /**
     * Of `level`'s layer K along `face`, the cells that may meet a ray
     * within `tangents` from an eye in `eyes`: [I0, I1, J0, J1]; none if
     * all are behind every eye.
     */
    fn rows(face: &Face, level: i32, k: i64, eyes: &Eyes, tangents: [f64; 4]) -> Option<[i32; 4]> {
        let (n, l) = ((1i64 << level) as f64, face.layer());
        let [lo_f, hi_f] = spread(face.f).map(f64::from);
        let c = k as f64 * n;
        // The cells' depths ahead of the eye, along the axis, nearest and farthest.
        let (z0, z1) = (((c + lo_f * n - eyes[2][1]) * l).max(0.0), (c + hi_f * n - eyes[2][0]) * l);
        if z1 <= 0.0 {
            return None;
        }
        let span = |lo: f64, hi: f64| (lo.min(lo * z0 / z1) * z1, hi.max(hi * z0 / z1) * z1);
        let (x0, x1) = span(tangents[0], tangents[1]);
        let (y0, y1) = span(tangents[2], tangents[3]);
        let [[a_lo, a_hi], [b_lo, b_hi], _] = *eyes;
        let [lo_r, hi_r] = spread(face.r).map(f64::from);
        Some([
            ((x0 / l + a_lo) / n - hi_r).floor() as i32,
            ((x1 / l + a_hi) / n - lo_r).ceil() as i32,
            ((y0 + b_lo) / n - 1.0).floor() as i32,
            ((y1 + b_hi) / n).ceil() as i32,
        ])
    }

    /**
     * The cells a face may compose (`composed`) from an eye anywhere in
     * `eyes`, its rays within `tangents`, kept by layer and row, each with a
     * `T` of the caller's: taken from the `old` pick where it had the cell,
     * at the same level of detail, so that moving a little keeps what is
     * known of most; what it does not take stays there.
     */
    pub fn pick<T: Default>(&self, face: &Face, eyes: Eyes, far: f64, tangents: [f64; 4], old: Option<&mut Picked<T>>) -> Picked<T> {
        let mut old = old.filter(|o| o.lod == *self && o.far == far && o.face == *face);
        let step = face.k();
        let mut levels = Vec::new();
        for level in 0..=self.top() {
            let Some(ks) = self.layers(face, level, &eyes, far) else {
                levels.push((0, Vec::new()));
                continue;
            };
            let first = *ks.start();
            let layers = ks
                .map(|k| {
                    let Some([a0, a1, b0, b1]) = Lod::rows(face, level, k, &eyes, tangents) else { return Layer::default() };
                    // Cells with I ≡ K (mod k).
                    let a = a0 + (k as i32 - a0).rem_euclid(step);
                    let (across, rows) = (((a1 - a) / step + 1).max(0) as usize, (b1 - b0 + 1).max(0) as usize);
                    let mut was = old.as_mut().and_then(|o| o.layer(level, k));
                    let mut layer = Layer { a, b: b0, step, across, rows, cells: Vec::new() };
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
        Picked { lod: *self, face: *face, far, eyes, tangents, levels }
    }

    /**
     * Whether `face` composes `cell` from `eye`, its rays within
     * `tangents`, to `far` ahead (world units): whether `composed` from
     * that eye alone visits it, at once.
     */
    pub fn composes(&self, face: &Face, cell: &CellChunk, eye: [f64; 3], far: f64, tangents: [f64; 4]) -> bool {
        let eyes = eye.map(|v| [v, v]);
        let level = cell.level;
        let [i, j, k] = cell.index(face);
        let in_rows = |rows: Option<[i32; 4]>, d: usize, v: i32| rows.is_some_and(|r| (r[2 * d]..=r[2 * d + 1]).contains(&v));
        self.layers(face, level, &eyes, far).is_some_and(|ks| ks.contains(&i64::from(k)))
            && in_rows(Lod::rows(face, level, k.into(), &eyes, tangents), 1, j)
            && in_rows(in_face(face, level, [j, k], &eyes, tangents).and_then(|t| Lod::rows(face, level, k.into(), &eyes, t)), 0, i)
    }

    /** The finest level claiming layer `layer` of grid 0 for an eye in layer `eye` (`reach`): the one drawing what is there. */
    pub fn claiming(&self, layer: f64, eye: i32) -> i32 {
        (self.finest()..self.top()).find(|&level| layer <= self.reach(level, eye) as f64).unwrap_or(self.top())
    }

    /**
     * The chunks `face` composes from an eye anywhere in `eyes` (Eyes):
     * the cells each level draws for one of them, from its layer to the
     * pick's `far` (world units) ahead along the axis, whose rays have x / z
     * and y / z within `tangents` (min x, max x, min y, max y); each with
     * its `T` in `picked`, which covers them (Picked::covers).
     */
    pub fn composed<T>(&self, picked: &mut Picked<T>, eyes: Eyes, tangents: [f64; 4], mut visit: impl FnMut(CellChunk, &mut T)) {
        let face = picked.face;
        for level in 0..=self.top() {
            let Some(ks) = self.layers(&face, level, &eyes, picked.far) else { continue };
            for k in ks {
                let Some([_, _, b0, b1]) = Lod::rows(&face, level, k, &eyes, tangents) else { continue };
                let layer = picked.layer(level, k).expect("covered");
                for r in within(layer.b, 1, layer.rows, b0, b1) {
                    let j = layer.b + r as i32;
                    let row = in_face(&face, level, [j, k as i32], &eyes, tangents);
                    let Some([a0, a1, _, _]) = row.and_then(|t| Lod::rows(&face, level, k, &eyes, t)) else { continue };
                    for s in within(layer.a, layer.step, layer.across, a0, a1) {
                        let i = layer.a + layer.step * s as i32;
                        visit(self.cell(&face, level, [i, j, k as i32]), &mut layer.cells[r * layer.across + s]);
                    }
                }
            }
        }
    }

    /**
     * How far from the eye, in grid `grid`'s sites, the chunks drawn of it
     * to `far` (world units) ahead read it, whichever way the eye turns and
     * whichever face; none if no level draws from it. A ray of a face meets
     * each cell drawn, and all the chunk reads (chunk_reads) is ahead of
     * the eye no deeper than its far side, and off the ray no more than
     * its width across.
     */
    pub fn grid_reach(&self, grid: usize, far: f64) -> Option<f64> {
        let faces = crate::face::faces();
        // One along an axis and one along an edge: the others are those turned.
        let eyes = [[0.0; 2], [0.0; 2], [-1.0, 0.0]];
        [faces[0], faces[6]]
            .iter()
            .flat_map(|face| {
                (self.finest()..=self.top()).filter(|&level| self.grid(level) as usize == grid).filter_map(move |level| {
                    let k = *self.layers(face, level, &eyes, far)?.end();
                    let (n, l) = (f64::from(1 << level), face.layer());
                    // The cell's far side, a cell deeper and a site more, as chunk_reads reads past it.
                    let ahead = ((k as f64 + 2.0 * f64::from(spread(face.f)[1])) * n + 2.0) * l;
                    let [sx, sy] = face.max_slant();
                    // Across: the ray's slant, and the cell and what is read around it, as wide.
                    let width = 2.0 * n + 2.0;
                    Some((ahead.max(0.0)).hypot(sx * ahead + width).hypot(sy * ahead + width) / f64::from(1 << grid))
                })
            })
            .reduce(f64::max)
    }
}

/**
 * Of `tangents`, those an edge face's rays (|x| + √2 |y| ≤ z) may have
 * through the voxels of the cells of `level` in row J of layer K from an
 * eye in `eyes`: x / z within ±(1 − √2 |y| / z) for their least |y| / z;
 * none if no ray of the face meets them. Along an axis a rectangle of
 * tangents is the face's rays.
 */
fn in_face(face: &Face, level: i32, [j, k]: [i32; 2], eyes: &Eyes, [x0, x1, y0, y1]: [f64; 4]) -> Option<[f64; 4]> {
    let n = f64::from(1 << level);
    let far = (f64::from(k) * n + f64::from(spread(face.f)[1]) * n + 1.0 - eyes[2][0]) * face.layer();
    if far <= 0.0 {
        return None;
    }
    if face.k() == 1 {
        return Some([x0, x1, y0, y1]);
    }
    let (low, high) = (f64::from(j) * n - 1.0 - eyes[1][1], f64::from(j) * n + n + 1.0 - eyes[1][0]);
    let least = if low > 0.0 { low } else { (-high).max(0.0) };
    let w = 1.0 - std::f64::consts::SQRT_2 * least / far;
    let (x0, x1) = (x0.max(-w), x1.min(w));
    (x0 <= x1).then_some([x0, x1, y0, y1])
}

/** Of `len` cells from `first`, `step` apart, those in [lo, hi], by index. */
fn within(first: i32, step: i32, len: usize, lo: i32, hi: i32) -> std::ops::Range<usize> {
    let index = |v: i32| v.clamp(0, len as i32) as usize;
    index((lo - first + step - 1).div_euclid(step))..index((hi - first).div_euclid(step) + 1)
}

/** A layer of a level's cells picked: from (I, J), `rows` rows of `across` cells, `step` apart across, and theirs. */
#[derive(Default)]
struct Layer<T> {
    a: i32,
    b: i32,
    step: i32,
    across: usize,
    rows: usize,
    cells: Vec<T>,
}

impl<T> Layer<T> {
    /** The cell (I, J) of its `at`-th. */
    fn site(&self, at: usize) -> [i32; 2] {
        [self.a + self.step * (at % self.across) as i32, self.b + (at / self.across) as i32]
    }

    /** The cell (I, J)'s, if it has it. */
    fn at(&mut self, i: i32, j: i32) -> Option<&mut T> {
        let d = i - self.a;
        if self.step == 0 || d.rem_euclid(self.step) != 0 {
            return None;
        }
        let (s, r) = (usize::try_from(d / self.step).ok()?, usize::try_from(j - self.b).ok()?);
        (s < self.across && r < self.rows).then(|| &mut self.cells[r * self.across + s])
    }
}

impl<T> Picked<T> {
    /** Whether it holds all `face` composes from `eye`, its rays within `tangents`, by `lod` to `far`. */
    pub fn covers(&self, lod: &Lod, face: &Face, far: f64, eye: [f64; 3], tangents: [f64; 4]) -> bool {
        let [w0, w1, w2, w3] = self.tangents;
        self.lod == *lod
            && self.face == *face
            && self.far == far
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
        let [i, j, k] = cell.index(&self.face);
        self.layer(cell.level, k.into())?.at(i, j)
    }

    /** Each of its cells, with its `T`. */
    pub fn into_each(self, mut visit: impl FnMut(CellChunk, T)) {
        for (level, (first, layers)) in (0..).zip(self.levels) {
            for (k, mut layer) in (first..).zip(layers) {
                for (at, t) in std::mem::take(&mut layer.cells).into_iter().enumerate() {
                    let [i, j] = layer.site(at);
                    visit(self.lod.cell(&self.face, level, [i, j, k as i32]), t);
                }
            }
        }
    }

    /** Layer K of `level`, if it has it. */
    fn layer(&mut self, level: i32, k: i64) -> Option<&mut Layer<T>> {
        let (first, layers) = self.levels.get_mut(usize::try_from(level).ok()?)?;
        layers.get_mut(usize::try_from(k - *first).ok()?)
    }
}

/**
 * A face's cells picked for an eye anywhere in `eyes` and rays within
 * `tangents` (Lod::pick), by `lod` to `far` ahead: per level, its first
 * layer K, and its layers, each with a `T` per cell.
 */
pub struct Picked<T> {
    lod: Lod,
    face: Face,
    far: f64,
    eyes: Eyes,
    tangents: [f64; 4],
    levels: Vec<(i64, Vec<Layer<T>>)>,
}

/** The eye anywhere in a box: [low, high] of each of its lattice coordinates along a face, in units of grid 0. */
pub type Eyes = [[f64; 2]; 3];

/** How far a cell's middle is from `eye` (by which the nearest are baked first), both along `face`, in units of grid 0. */
pub fn distance(face: &Face, cell: &CellChunk, eye: [f64; 3]) -> f64 {
    let n = f64::from(1 << cell.level);
    let middle = face.middle();
    let [a, b, c] = std::array::from_fn(|d| (f64::from(cell.index(face)[d]) + middle[d]) * n - eye[d]);
    let l = face.layer();
    ((a * l).powi(2) + b * b + (c * l).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face::{FACES, faces};

    use std::collections::HashSet;

    /** The cells a face composes from `eye`, its rays within `tangents`, of `picked`: each once. */
    fn composed_of<T>(lod: &Lod, picked: &mut Picked<T>, eye: [f64; 3], tangents: [f64; 4]) -> HashSet<CellChunk> {
        let mut cells = HashSet::new();
        lod.composed(picked, eye.map(|v| [v, v]), tangents, |cell, _| assert!(cells.insert(cell), "{cell:?} twice"));
        cells
    }

    /** The cells `face` composes from `eye`, its rays within `tangents`, picked there. */
    fn composed_from(lod: &Lod, face: &Face, eye: [f64; 3], far: f64, tangents: [f64; 4]) -> HashSet<CellChunk> {
        composed_of(lod, &mut lod.pick::<()>(face, eye.map(|v| [v, v]), far, tangents, None), eye, tangents)
    }

    /** A face's rays' tangents, all. */
    fn all(face: &Face) -> [f64; 4] {
        let [x, y] = face.max_slant();
        [-x, x, -y, y]
    }

    #[test]
    fn a_pick_composes_from_an_eye_in_its_box_what_one_made_there_would_and_keeps_what_it_knew() {
        let (lod, far) = (Lod { shift: 3, q: 10, medium: false }, 300.0);
        for face in [faces()[0], faces()[9]] {
            let [x, y] = face.max_slant();
            let (wide, tangents) = ([-x, x * 0.9, -y * 0.8, y], [-x * 0.9, x * 0.8, -y * 0.7, y * 0.9]);
            let around = |centre: [f64; 3]| centre.map(|v| [v - 2.0, v + 2.0]);
            let centre = [3.3, -1.2, 7.8];
            let mut old: Picked<Option<CellChunk>> = lod.pick(&face, around(centre), far, wide, None);
            let mut known = HashSet::new();
            for d in [[-2.0, 2.0, -2.0], [1.9, -0.4, 1.99], [0.0, 0.0, 0.0], [-1.1, -2.0, 0.6]] {
                let eye = std::array::from_fn(|i| centre[i] + d[i]);
                let there = composed_from(&lod, &face, eye, far, tangents);
                assert!(there.len() > 300, "{}", there.len());
                assert_eq!(composed_of(&lod, &mut old, eye, tangents), there, "eye {eye:?}");
                lod.composed(&mut old, eye.map(|v| [v, v]), tangents, |cell, seen| *seen = Some(cell));
                known.extend(there);
            }
            // Picked again a little farther on: what it knew of a cell is the new pick's, where `at` finds it, or stays in the old.
            let moved = centre.map(|v| v + 2.5);
            assert!(!old.covers(&lod, &face, far, moved, tangents));
            let mut new = lod.pick(&face, around(moved), far, wide, Some(&mut old));
            assert!(known.iter().all(|cell| new.at(cell).is_none_or(|seen| *seen == Some(*cell))));
            let mut kept = HashSet::new();
            new.into_each(|cell, seen| assert!(seen.is_none_or(|s| s == cell && kept.insert(cell)), "{cell:?} has {seen:?}"));
            assert!(kept.len() * 2 > known.len(), "{} of {} kept", kept.len(), known.len());
            old.into_each(|cell, seen| assert!(seen.is_none_or(|s| s == cell && kept.insert(cell)), "{cell:?} has {seen:?}"));
            assert_eq!(kept, known);
        }
    }

    #[test]
    fn a_pick_composes_for_its_whole_box_all_that_an_eye_in_it_composes_cell_by_cell() {
        let (lod, far) = (Lod { shift: 3, q: 8, medium: false }, 300.0);
        for face in [faces()[3], faces()[13]] {
            let wide = all(&face);
            let [x, y] = face.max_slant();
            let tangents = [-x * 0.8, x * 0.7, -y * 0.6, y * 0.75];
            let centre = [-4.1, 2.3, 11.6];
            let mut picked: Picked<()> = lod.pick(&face, centre.map(|v| [v - 3.0, v + 3.0]), far, wide, None);
            let (eyes, bounds) = picked.bounds();
            let mut every = HashSet::new();
            lod.composed(&mut picked, eyes, bounds, |cell, _| assert!(every.insert(cell), "{cell:?} twice"));
            for d in [[-3.0, 3.0, -3.0], [2.9, -1.4, 2.99], [0.0, 0.0, 0.0], [-1.7, -3.0, 0.4]] {
                let eye = std::array::from_fn(|i| centre[i] + d[i]);
                let there = composed_of(&lod, &mut picked, eye, tangents);
                let composes: HashSet<CellChunk> =
                    every.iter().copied().filter(|cell| lod.composes(&face, cell, eye, far, tangents)).collect();
                assert_eq!(composes, there, "eye {eye:?}: composes, cell by cell, as composed");
                assert!(
                    there.len() > 300 && there.is_subset(&every),
                    "eye {eye:?}: {} of {} not in the box's",
                    there.difference(&every).count(),
                    there.len()
                );
            }
        }
    }

    /** Whether site p of grid 0 is in `face`'s rays from `eye` (its lattice coordinates) and within `far`, well within. */
    fn in_view(face: &Face, eye: [f64; 3], p: [i32; 3], far: f64) -> bool {
        let middle = face.middle();
        let l = face.layer();
        let abc = face.of(p);
        let x = [(abc[0] as f64 + middle[0] - eye[0]) * l, abc[1] as f64 + middle[1] - eye[1], (abc[2] as f64 + middle[2] - eye[2]) * l];
        let z = x[2] - 0.01;
        face.holds([x[0], x[1], z]) && z > 0.0 && (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() <= far
    }

    /** The cell of `level` holding site p of grid 0. */
    fn cell_of(lod: &Lod, level: i32, p: [i32; 3]) -> CellChunk {
        CellChunk { level, atoms: lod.grid(level), cell: p.map(|v| v >> level), medium: lod.medium }
    }

    #[test]
    fn every_site_ahead_is_drawn_first_at_the_finest_level_claiming_it() {
        let far = 120.0;
        for (n, face) in faces().iter().enumerate() {
            let lod = Lod { shift: 1 + (n as i32 % 3), q: 4 + n as i32 % 3, medium: false };
            let eye = [0.3 + n as f64, -2.6 + (n % 4) as f64, 1.7 - 3.3 * (n % 3) as f64];
            let camera = eye[2].floor() as i32;
            let chunks = composed_from(&lod, face, eye, far, all(face));
            let levels: std::collections::BTreeSet<i32> = chunks.iter().map(|c| c.level).collect();
            assert!(levels.len() >= 3, "face {n}: levels {levels:?}");
            let mut ahead = 0;
            for x in -80..=80 {
                for y in -30..=30 {
                    for z in -80..=80 {
                        let p = [x, y, z];
                        if !in_view(face, eye, p, far) {
                            continue;
                        }
                        let holding: Vec<i32> = levels.iter().copied().filter(|&l| chunks.contains(&cell_of(&lod, l, p))).collect();
                        let near = face.of(p)[2] <= camera + NEAR;
                        let claimed = |l: i32| i64::from(face.of(p.map(|v| v >> l))[2] << l) <= lod.reach(l, camera);
                        let finest = (0..=lod.top()).find(|&l| claimed(l));
                        if !near {
                            assert_eq!(holding.first().copied(), finest, "face {n} site {p:?}");
                        }
                        ahead += 1;
                    }
                }
            }
            assert!(ahead > 3000, "{ahead}");
        }
    }

    /**
     * The levels a ray of `face` from `eye` meets, where each shows (the
     * finest of the chunks holding a point): how many times it goes from
     * one to another, and how many of those to a finer one.
     */
    fn crossings(lod: &Lod, face: &Face, eye: [f64; 3], far: f64) -> (usize, usize) {
        let chunks = composed_from(lod, face, eye, far, all(face));
        let l = face.layer();
        let shows = |x: [f64; 3]| {
            let p = face.point([x[0] * l, x[1], x[2] * l]).map(|v| v.floor() as i32);
            (0..=lod.top()).find(|&level| chunks.contains(&cell_of(lod, level, p)))
        };
        let [sx, sy] = face.max_slant();
        let (mut met, mut wrong) = (0, 0);
        for r in 0..200 {
            let (u, v) = ((r * 37 % 101) as f64 / 101.0 * 2.0 - 1.0, (r * 53 % 97) as f64 / 97.0 * 2.0 - 1.0);
            let ray = [u * sx, v * sy, 1.0];
            if !face.holds(ray) {
                continue;
            }
            let dir = [ray[0] / l, ray[1], 1.0 / l];
            let mut last: Option<i32> = None;
            for t in 6..1600 {
                let x: [f64; 3] = std::array::from_fn(|d| eye[d] + dir[d] * t as f64 * 0.07);
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
        for (n, face) in faces().iter().enumerate() {
            let lod = Lod { shift: 2, q: 4 + n as i32 % 3, medium: false };
            let eye = [0.37 * n as f64 - 2.0, 0.21 * (n % 5) as f64 - 0.4, 0.29 * (n % 7) as f64 - 3.0 * n as f64];
            let (m, w) = crossings(&lod, face, eye, 120.0);
            (met, wrong) = (met + m, wrong + w);
        }
        assert!(met > 1000 && wrong == 0, "{wrong} of {met} crossings to a finer level");
    }

    #[test]
    fn the_chunks_of_each_grid_read_it_within_its_reach() {
        for (shift, q, far) in [(3, 20, 4000.0), (3, 4, 1000.0), (2, 7, 1000.0), (1, 5, 300.0)] {
            let whole = Lod { shift, q, medium: false };
            for lod in [whole, whole.for_medium()] {
                let mut most = [0.0f64; GRID_LEVELS];
                for face in faces().iter().take(FACES) {
                    let eye = [0.37, -0.4, -0.2];
                    let eye_world = face.point([eye[0] * face.layer(), eye[1], eye[2] * face.layer()]);
                    let mut picked = lod.pick::<()>(face, eye.map(|v| [v, v]), far, all(face), None);
                    lod.composed(&mut picked, eye.map(|v| [v, v]), all(face), |cell, _| {
                        let (g, min, max) = crate::chunk_reads(face, &cell);
                        let s = f64::from(1 << g);
                        for corner in 0..8 {
                            let p: [f64; 3] = std::array::from_fn(|d| {
                                (if corner >> d & 1 == 0 { f64::from(min[d]) } else { f64::from(max[d]) + 1.0 }) * s - eye_world[d]
                            });
                            most[g] = most[g].max((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() / s);
                        }
                    });
                }
                for (g, most) in most.into_iter().enumerate() {
                    let reach = lod.grid_reach(g, far);
                    assert!(
                        reach.is_some_and(|r| most <= r && most > r / 3.0) || reach.is_none() && most == 0.0,
                        "{lod:?}, far {far}: grid {g} read {most:.0} sites far, its reach {reach:?}"
                    );
                }
            }
        }
    }
}
