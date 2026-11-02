//! Which parts of the screen nearer layers already cover, so chunks behind
//! them need not be drawn or baked. Runs on the main thread, in its own
//! instance of the module; occlusion.ts drives it through the exports at
//! the bottom. See docs/architecture.md, section 4.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::SQRT_2;

use crate::fill::{BLOCK, DRAWN, SOLID};

/** Side of a coverage cell, in CSS pixels. */
const CELL: usize = 16;
/** Side of a sub-block of a cell, in CSS pixels: a cell has SUBS × SUBS of them. */
const SUB: usize = 4;
const SUBS: usize = CELL / SUB;
const SHIFT: u32 = SUBS.trailing_zeros();
const MASK: usize = SUBS - 1;
/** A cell's bits with all its sub-blocks covered. */
const FULL: u16 = u16::MAX;
/**
 * A box counts as covered only with this many covered CSS pixels around
 * it, so a small move does not uncover chunks right away.
 */
const MARGIN: f64 = 8.0;
/** Tiles across and down a chunk's image in Chunk::tiles. */
const TILES: usize = 4;
/** Points nearer the camera plane than this are cut off before projecting (NEAR_PLANE in view.ts). */
const NEAR_PLANE: f64 = 1e-3;
/** Renders a chunk's blocks are kept without being used. */
const KEEP: u64 = 16;

/** Bit 0 of a cell's rows of sub-blocks a to b. */
fn rows_mask(a: usize, b: usize) -> u16 {
    (a..=b).fold(0, |mask, r| mask | 1 << (r * SUBS))
}

/** The bits of a cell's columns of sub-blocks a to b, in row 0. */
fn columns_mask(a: usize, b: usize) -> u16 {
    ((1 << (b + 1)) - 1) & !((1 << a) - 1)
}

type Vec3 = [f64; 3];

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/**
 * Clips a convex polygon to the half-space distance(p) ≥ 0 (Sutherland–
 * Hodgman), for a linear `distance` (clip in view.ts).
 */
fn clip(polygon: &[Vec3], distance: impl Fn(Vec3) -> f64) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (k, &p) in polygon.iter().enumerate() {
        let q = polygon[(k + 1) % polygon.len()];
        let (dp, dq) = (distance(p), distance(q));
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp < 0.0) != (dq < 0.0) {
            let t = dp / (dp - dq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t]);
        }
    }
    out
}

/** A rectangle: on screen, CSS pixels from the top-left, or of tangents. */
#[derive(Clone, Copy)]
struct Bounds {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

impl Bounds {
    const EMPTY: Bounds = Bounds { min_x: f64::INFINITY, max_x: f64::NEG_INFINITY, min_y: f64::INFINITY, max_y: f64::NEG_INFINITY };

    fn add(&mut self, x: f64, y: f64) {
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_y = self.min_y.min(y);
        self.max_y = self.max_y.max(y);
    }
}

/**
 * Bounds of x / z and y / z of the face's rays (inFace in view.ts) within
 * the convex cone spanned by `rays`, in order around it; None if it has
 * none of them, or only along an edge (tangentBox in view.ts).
 */
fn tangent_box(rays: &[Vec3]) -> Option<Bounds> {
    let mut rays = rays.to_vec();
    for s in [1.0, -1.0] {
        for t in [1.0, -1.0] {
            rays = clip(&rays, |p| p[2] - s * p[0] - t * SQRT_2 * p[1]);
        }
    }
    let tangents: Vec<(f64, f64)> = rays.iter().map(|r| (r[0] / r[2], r[1] / r[2])).collect();
    // Shoelace area: a cone the face only touches along an edge shows nothing of it.
    let area = tangents.iter().enumerate().fold(0.0, |sum, (k, p)| {
        let q = tangents[(k + 1) % tangents.len()];
        sum + p.0 * q.1 - q.0 * p.1
    });
    if area.is_nan() || area.abs() <= 1e-9 {
        return None;
    }
    let mut bounds = Bounds::EMPTY;
    for &(x, y) in &tangents {
        bounds.add(x, y);
    }
    Some(bounds)
}

/** The view of one face: viewport in CSS pixels, focal length in pixels, and the camera's axes in the face's coordinates. */
#[derive(Clone, Copy, PartialEq)]
struct Look {
    width: f64,
    height: f64,
    focal: f64,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
}

impl Look {
    /** Direction through the screen point (sx, sy), in the face's coordinates (screenRay in view.ts). */
    fn ray(&self, sx: f64, sy: f64) -> Vec3 {
        let (a, b) = (sx - self.width / 2.0, self.height / 2.0 - sy);
        std::array::from_fn(|k| self.forward[k] * self.focal + self.right[k] * a + self.up[k] * b)
    }

    /**
     * The screen bounding box of the part in front of the camera of a
     * camera-relative rectangle [left, right] × [bottom, top] on the face's
     * plane z = distance, or None if none of it is in front (screenBox in
     * view.ts).
     */
    fn screen_box(&self, left: f64, right: f64, bottom: f64, top: f64, distance: f64) -> Option<Bounds> {
        let Look { right: r, up: u, forward, focal: f, .. } = *self;
        let (cx, cy) = (self.width / 2.0, self.height / 2.0);
        // Depth along the camera's axis of the corners: linear in x and y.
        let zl = forward[0] * left + forward[2] * distance;
        let zr = forward[0] * right + forward[2] * distance;
        let zb = forward[1] * bottom;
        let zt = forward[1] * top;
        let mut bounds = Bounds::EMPTY;
        if zl.min(zr) + zb.min(zt) > NEAR_PLANE {
            // All in front (the common case): project the corners directly.
            for (x, zx) in [(left, zl), (right, zr)] {
                for (y, zy) in [(bottom, zb), (top, zt)] {
                    let z = zx + zy;
                    bounds.add(
                        cx + ((r[0] * x + r[1] * y + r[2] * distance) * f) / z,
                        cy - ((u[0] * x + u[1] * y + u[2] * distance) * f) / z,
                    );
                }
            }
            return Some(bounds);
        }
        let corners = [[left, bottom, distance], [right, bottom, distance], [right, top, distance], [left, top, distance]];
        let mut any = false;
        for p in clip(&corners, |p| dot(p, forward) - NEAR_PLANE) {
            let z = dot(p, forward);
            if z > 0.0 {
                bounds.add(cx + (dot(p, r) * f) / z, cy - (dot(p, u) * f) / z);
                any = true;
            }
        }
        any.then_some(bounds)
    }

    /** Whether a screen box overlaps the viewport (onScreen in view.ts). */
    fn on_screen(&self, b: &Bounds) -> bool {
        b.min_x < self.width && b.max_x > 0.0 && b.min_y < self.height && b.max_y > 0.0
    }
}

/** What Coverage keeps of a face per view: see Coverage::shape. */
struct Shape {
    look: Look,
    /**
     * Per sub-block (see sub_block): bounds of x / z and y / z of the
     * face's rays through it (min x, max x, min y, max y).
     */
    tangents: Vec<[f64; 4]>,
    /** The same per cell wholly inside the face; NaN for others. */
    cell_tangents: Vec<[f64; 4]>,
    /**
     * Per cell, a bit per sub-block outside the face. The tangents of cells
     * wholly outside (all bits) are left from an earlier view: never read.
     */
    outside: Vec<u16>,
}

/** Which of a face's sides (see inFace in view.ts) a ray is outside of, a bit each. */
fn sides([x, y, z]: Vec3) -> u8 {
    (z < x + SQRT_2 * y) as u8 | ((z < -x + SQRT_2 * y) as u8) << 1 | ((z < x - SQRT_2 * y) as u8) << 2 | ((z < -x - SQRT_2 * y) as u8) << 3
}

/** What Coverage needs of a baked chunk. */
struct Chunk {
    width: usize,
    height: usize,
    scale: f64,
    /** Bounds of the solid blocks in pixels, [left, right) × [top, bottom); empty if there are none. */
    solid: [f64; 4],
    /**
     * Per corner of its blocks (see solid_blocks), in rows of
     * ceil(width / BLOCK) + 1: the number of solid blocks above and to
     * the left of it.
     */
    sums: Vec<u32>,
    /**
     * The chunk's image cut into TILES × TILES tiles of whole blocks: per
     * tile, the bounds in pixels (left, right, top, bottom) of its blocks
     * with a pixel drawn, left ≥ right if none.
     */
    tiles: [[usize; 4]; TILES * TILES],
    /** The render it was last used in. */
    used: u64,
}

impl Chunk {
    /** A chunk of width × height pixels drawn at `scale`, from its blocks (see solid_blocks). */
    fn new(width: usize, height: usize, scale: f64, blocks: &[u8]) -> Chunk {
        let cols = width.div_ceil(BLOCK);
        let rows = height.div_ceil(BLOCK);
        let mut sums = vec![0; (cols + 1) * (rows + 1)];
        let mut tiles = [[width, 0, height, 0]; TILES * TILES];
        let mut solid = [width, 0, height, 0];
        for by in 0..rows {
            let mut row = 0;
            for bx in 0..cols {
                let bits = blocks[by * cols + bx];
                row += u32::from(bits & SOLID);
                sums[(by + 1) * (cols + 1) + bx + 1] = sums[by * (cols + 1) + bx + 1] + row;
                // The bounds of the block, cut by the image's edge.
                let bounds = [bx * BLOCK, width.min((bx + 1) * BLOCK), by * BLOCK, height.min((by + 1) * BLOCK)];
                let grow = |b: &mut [usize; 4]| *b = [b[0].min(bounds[0]), b[1].max(bounds[1]), b[2].min(bounds[2]), b[3].max(bounds[3])];
                if bits & SOLID != 0 {
                    grow(&mut solid);
                }
                if bits & DRAWN != 0 {
                    grow(&mut tiles[by * TILES / rows * TILES + bx * TILES / cols]);
                }
            }
        }
        Chunk { width, height, scale, solid: solid.map(|v| v as f64), sums, tiles, used: 0 }
    }

    /**
     * The numbers of solid blocks, and of all blocks, under the rectangle
     * [l, r] × [b, t] of the plane, the chunk's top-left corner at (x, y),
     * with a pixel around (minified, the texture is smoothed over
     * neighbours). A smaller rectangle has fewer blocks under it.
     */
    fn under(&self, x: f64, y: f64, l: f64, r: f64, b: f64, t: f64) -> (usize, usize) {
        let Chunk { width, height, scale, .. } = *self;
        let block = BLOCK as f64;
        let x1 = ((width as f64).min(((r - x) * scale).ceil() + 1.0) / block).ceil() as usize;
        let y1 = ((height as f64).min(((y - b) * scale).ceil() + 1.0) / block).ceil() as usize;
        let x0 = ((((l - x) * scale).floor() - 1.0).max(0.0) as usize / BLOCK).min(x1);
        let y0 = ((((y - t) * scale).floor() - 1.0).max(0.0) as usize / BLOCK).min(y1);
        (self.solid_in(x0, x1, y0, y1), (x1 - x0) * (y1 - y0))
    }

    /** The number of solid blocks in [x0, x1) × [y0, y1), x0 ≤ x1 and y0 ≤ y1. */
    fn solid_in(&self, x0: usize, x1: usize, y0: usize, y1: usize) -> usize {
        let at = |x: usize, y: usize| self.sums[y * (self.width.div_ceil(BLOCK) + 1) + x] as usize;
        at(x1, y1) + at(x0, y0) - at(x0, y1) - at(x1, y0)
    }
}

/**
 * The number of a sub-block, by its cell and its bit in the cell (see
 * Coverage::bits): cell by cell, so a cell's sub-blocks are together in memory.
 */
fn sub_block(cell: usize, bit: usize) -> usize {
    cell * SUBS * SUBS + bit
}

/** The range of sub-blocks (i0, i1, j0, j1, inclusive) a screen box with `margin` around it overlaps, clamped to sub_cols × sub_rows. */
fn sub_blocks(b: &Bounds, margin: f64, sub_cols: usize, sub_rows: usize) -> [usize; 4] {
    let clamp = |v: f64, n: usize| ((v / SUB as f64).floor().max(0.0) as usize).min(n - 1);
    [
        clamp(b.min_x - margin, sub_cols),
        clamp(b.max_x + margin, sub_cols),
        clamp(b.min_y - margin, sub_rows),
        clamp(b.max_y + margin, sub_rows),
    ]
}

/** The first cell from k (by rows of cols + 1) the current face could still cover, or the row's stop. */
fn next(open: &mut [usize], mut k: usize) -> usize {
    while open[k] != k {
        open[k] = open[open[k]];
        k = open[k];
    }
    k
}

/**
 * The screen in cells of CELL pixels, each covered or not, for the face
 * being drawn. Each pixel shows one face (but for a hair at seams), so a
 * face only needs its own part of a cell covered, and cells outside it
 * count as covered. Conservative: a cell is covered once each of its
 * sub-blocks of SUB pixels, the face's part of it, is covered by one layer
 * drawn so far (by opaque pixels of that layer's chunks); different
 * sub-blocks may be covered by different layers. Faces come one by one,
 * each with its layers near to far.
 */
#[derive(Default)]
pub struct Coverage {
    width: f64,
    height: f64,
    focal: f64,
    cols: usize,
    rows: usize,
    // Sub-blocks across and down: cols · SUBS, rows · SUBS.
    sub_cols: usize,
    sub_rows: usize,
    covered: Vec<bool>,
    // Per cell, a bit per sub-block (row by row) the current face covers, or that is outside it.
    bits: Vec<u16>,
    // Per cell, by rows of cols + 1 (the last one a stop): itself if the
    // current face could still cover it, else towards the next such cell in
    // its row (see next()).
    open: Vec<usize>,
    // Per face, its shape for the latest view it was seen through, and the current face.
    shapes: Vec<Option<Shape>>,
    face: Option<usize>,
    // Tangents (x / z, y / z) of the rays through the sub-blocks' corners, and the face's sides each is outside of.
    rays: Vec<f64>,
    outside: Vec<u8>,
    // The same sides, of the rays through the cells' corners.
    cell_sides: Vec<u8>,
    // Per sub-block (see sub_block), for the layer being covered: area of
    // the plane under it proven opaque so far, and the sub-blocks with some.
    area: Vec<f64>,
    touched: Vec<usize>,
    // Chunks by handle (see occlusion.ts), and the number of renders so far.
    chunks: HashMap<u32, Chunk>,
    renders: u64,
}

impl Coverage {
    /** Starts a render, of a viewport of width × height CSS pixels; buffers are kept while its size stays. */
    pub fn set_view(&mut self, width: f64, height: f64, focal: f64) {
        self.renders += 1;
        let renders = self.renders;
        self.chunks.retain(|_, chunk| chunk.used + KEEP > renders);
        (self.width, self.height, self.focal) = (width, height, focal);
        self.face = None;
        let cols = ((width / CELL as f64).ceil() as usize).max(1);
        let rows = ((height / CELL as f64).ceil() as usize).max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        (self.cols, self.rows) = (cols, rows);
        self.sub_cols = cols * SUBS;
        self.sub_rows = rows * SUBS;
        let cells = cols * rows;
        let subs = self.sub_cols * self.sub_rows;
        self.shapes.clear();
        self.covered = vec![false; cells];
        self.bits = vec![0; cells];
        self.open = vec![0; (cols + 1) * rows];
        self.area = vec![0.0; subs];
        let corners = (self.sub_cols + 1) * (self.sub_rows + 1);
        self.rays = vec![0.0; corners * 2];
        self.outside = vec![0; corners];
        self.cell_sides = vec![0; (cols + 1) * (rows + 1)];
    }

    /** Starts face number `face` (of FACES in view.ts), the camera's axes in its coordinates; its layers follow, near to far. */
    pub fn set_face(&mut self, face: usize, right: Vec3, up: Vec3, forward: Vec3) {
        let look = Look { width: self.width, height: self.height, focal: self.focal, right, up, forward };
        if self.shapes.len() <= face {
            self.shapes.resize_with(face + 1, || None);
        }
        // What depends only on the view and the face is kept while they stay (moving, not turning).
        if self.shapes[face].as_ref().is_none_or(|shape| shape.look != look) {
            let old = self.shapes[face].take();
            self.shapes[face] = Some(self.shape(look, old));
        }
        self.face = Some(face);
        let Coverage { cols, rows, ref mut open, ref mut covered, ref mut bits, .. } = *self;
        bits.copy_from_slice(&self.shapes[face].as_ref().expect("just made").outside);
        for j in 0..rows {
            for i in 0..cols {
                let c = j * cols + i;
                let k = j * (cols + 1) + i;
                covered[c] = bits[c] == FULL;
                open[k] = if covered[c] { k + 1 } else { k };
            }
            open[j * (cols + 1) + cols] = j * (cols + 1) + cols;
        }
    }

    /**
     * The tangents of a face's rays through each sub-block and cell, and per
     * cell its sub-blocks outside the face; in the buffers of `old`, if any.
     */
    fn shape(&mut self, look: Look, old: Option<Shape>) -> Shape {
        let Coverage { cols, rows, sub_cols, sub_rows, ref mut rays, ref mut outside, ref mut cell_sides, .. } = *self;
        let (mut tangents, mut cell_tangents, mut bits) = match old {
            Some(Shape { tangents, cell_tangents, outside, .. }) => (tangents, cell_tangents, outside),
            None => (vec![[0.0; 4]; sub_cols * sub_rows], vec![[0.0; 4]; cols * rows], vec![0u16; cols * rows]),
        };
        // A side is a half-plane of the screen, so a cell with its corners
        // outside one side is outside the face, and one with all its corners
        // inside is inside: only cells it crosses or holds are looked into.
        for (n, side) in cell_sides.iter_mut().enumerate() {
            *side = sides(look.ray((n % (cols + 1) * CELL) as f64, (n / (cols + 1) * CELL) as f64));
        }
        let cell_corners = |c: usize| {
            let n = c + c / cols;
            [cell_sides[n], cell_sides[n + 1], cell_sides[n + cols + 1], cell_sides[n + cols + 2]]
        };
        for (c, b) in bits.iter_mut().enumerate() {
            let o = cell_corners(c);
            *b = if o[0] & o[1] & o[2] & o[3] != 0 { FULL } else { 0 };
        }
        // Tangents of the rays through those cells' sub-block corners, and the sides each is outside of.
        // Only within the span [from, to) of the cells looked into in their rows ([cols, 0) if none).
        let spans: Vec<[usize; 2]> = bits
            .chunks(cols)
            .map(|row| {
                [row.iter().position(|&b| b != FULL).unwrap_or(cols), row.iter().rposition(|&b| b != FULL).map_or(0, |last| last + 1)]
            })
            .collect();
        let corners = sub_cols + 1;
        for sj in 0..=sub_rows {
            let (j0, j1) = (sj.saturating_sub(1) >> SHIFT, (sj >> SHIFT).min(rows - 1));
            let (from, to) = (spans[j0][0].min(spans[j1][0]), spans[j0][1].max(spans[j1][1]));
            for si in from * SUBS..=to * SUBS {
                let (i0, i1) = (si.saturating_sub(1) >> SHIFT, (si >> SHIFT).min(cols - 1));
                if [j0 * cols + i0, j0 * cols + i1, j1 * cols + i0, j1 * cols + i1].iter().all(|&c| bits[c] == FULL) {
                    continue;
                }
                let n = sj * corners + si;
                let ray = look.ray((si * SUB) as f64, (sj * SUB) as f64);
                (rays[2 * n], rays[2 * n + 1], outside[n]) = (ray[0] / ray[2], ray[1] / ray[2], sides(ray));
            }
        }
        // The bounds of a rectangle's tangents, of corners k (top-left) to k + d + corners · e, all inside the face:
        // its tangents are extreme at its corners (ratios of linear functions of the screen position).
        let bounds = |k: usize, d: usize| {
            let q = [k, k + d, k + d * corners, k + d * (corners + 1)];
            let (xs, ys) = (q.map(|n| rays[2 * n]), q.map(|n| rays[2 * n + 1]));
            [
                xs.into_iter().fold(f64::INFINITY, f64::min),
                xs.into_iter().fold(f64::NEG_INFINITY, f64::max),
                ys.into_iter().fold(f64::INFINITY, f64::min),
                ys.into_iter().fold(f64::NEG_INFINITY, f64::max),
            ]
        };
        let ray_at = |n: usize| look.ray((n % corners * SUB) as f64, (n / corners * SUB) as f64);
        for c in 0..cols * rows {
            if bits[c] == FULL {
                continue;
            }
            let corner = c / cols * SUBS * corners + c % cols * SUBS;
            for bit in 0..SUBS * SUBS {
                let k = corner + (bit >> SHIFT) * corners + (bit & MASK);
                let s = sub_block(c, bit);
                let o = [outside[k], outside[k + 1], outside[k + corners], outside[k + corners + 1]];
                // All corners inside: bounded by its corners; all outside one side: outside the face; otherwise clip.
                if o[0] | o[1] | o[2] | o[3] == 0 {
                    tangents[s] = bounds(k, 1);
                    continue;
                }
                let clipped =
                    if o[0] & o[1] & o[2] & o[3] == 0 { tangent_box(&[k, k + 1, k + corners + 1, k + corners].map(ray_at)) } else { None };
                match clipped {
                    Some(b) => tangents[s] = [b.min_x, b.max_x, b.min_y, b.max_y],
                    None => bits[c] |= 1 << bit,
                }
            }
            // A cell wholly inside the face, for covering it at once (NaN if not).
            cell_tangents[c] = if cell_corners(c) == [0; 4] { bounds(corner, SUBS) } else { [f64::NAN; 4] };
        }
        Shape { look, tangents, cell_tangents, outside: bits }
    }

    /** The current face's shape. */
    fn current(&self) -> Option<&Shape> {
        self.shapes.get(self.face?)?.as_ref()
    }

    /** Whether a screen box, with MARGIN around it, lies on covered sub-blocks. */
    pub fn hides(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
        self.hides_box(&Bounds { min_x, max_x, min_y, max_y })
    }

    fn hides_box(&self, b: &Bounds) -> bool {
        let [si0, si1, sj0, sj1] = sub_blocks(b, MARGIN, self.sub_cols, self.sub_rows);
        for j in sj0 >> SHIFT..=sj1 >> SHIFT {
            // The cell's rows of sub-blocks in the box, as a mask of a column of them.
            let rows = rows_mask(sj0.saturating_sub(j * SUBS), (sj1 - j * SUBS).min(SUBS - 1));
            for i in si0 >> SHIFT..=si1 >> SHIFT {
                let c = j * self.cols + i;
                if self.covered[c] {
                    continue;
                }
                let mask = rows * columns_mask(si0.saturating_sub(i * SUBS), (si1 - i * SUBS).min(SUBS - 1));
                if self.bits[c] & mask != mask {
                    return false;
                }
            }
        }
        true
    }

    /**
     * Whether chunk `handle` placed at (x, y) on the plane z = distance
     * (see Placement in renderer.ts) is hidden: each tile of its image with
     * a pixel drawn (see Chunk::tiles) is hidden (see hides) or off screen.
     */
    pub fn hides_chunk(&self, handle: u32, x: f64, y: f64, distance: f64) -> bool {
        let Some(look) = self.current().map(|shape| shape.look) else { return false };
        let Some(chunk) = self.chunks.get(&handle) else { return false };
        let scale = chunk.scale;
        chunk.tiles.iter().filter(|tile| tile[0] < tile[1]).all(|&[l, r, t, b]| {
            let (l, r, t, b) = (l as f64 / scale, r as f64 / scale, t as f64 / scale, b as f64 / scale);
            look.screen_box(x + l, x + r, y - b, y - t, distance).is_none_or(|b| !look.on_screen(&b) || self.hides_box(&b))
        })
    }

    /**
     * Adds what chunk `handle` of a layer of the current face, placed at
     * (x, y) on the plane z = distance, draws opaque; its grid cell (see
     * chunkCell) starts at (cell_left, cell_bottom), camera-relative. The
     * layer ends with end_layer.
     */
    #[allow(clippy::too_many_arguments)]
    pub fn cover(
        &mut self,
        handle: u32,
        x: f64,
        y: f64,
        cell_left: f64,
        cell_bottom: f64,
        cell_width: f64,
        cell_height: f64,
        distance: f64,
    ) {
        let Some(face) = self.face else { return };
        let Coverage {
            cols,
            sub_cols,
            sub_rows,
            ref shapes,
            ref mut bits,
            ref mut covered,
            ref mut open,
            ref mut area,
            ref mut touched,
            ..
        } = *self;
        let Some(Shape { look, tangents, cell_tangents, .. }) = &shapes[face] else { return };
        let Some(chunk) = self.chunks.get(&handle) else { return };
        let scale = chunk.scale;
        let [solid_left, solid_right, solid_top, solid_bottom] = chunk.solid;
        // The all-drawn blocks, clipped to the chunk's grid cell so pieces of
        // a sub-block from different chunks don't overlap.
        let left = (x + solid_left / scale).max(cell_left);
        let right = (x + solid_right / scale).min(cell_left + cell_width);
        let bottom = (y - solid_bottom / scale).max(cell_bottom);
        let top = (y - solid_top / scale).min(cell_bottom + cell_height);
        if left >= right || bottom >= top {
            return;
        }
        let Some(b) = look.screen_box(left, right, bottom, top, distance) else { return };
        let under = |l, r, b, t| chunk.under(x, y, l, r, b, t);
        let all_solid = |l, r, b, t| {
            let (solid, all) = under(l, r, b, t);
            solid == all
        };
        let [si0, si1, sj0, sj1] = sub_blocks(&b, 0.0, sub_cols, sub_rows);
        for j in sj0 >> SHIFT..=sj1 >> SHIFT {
            let sj_from = sj0.max(j * SUBS);
            let sj_to = sj1.min(j * SUBS + SUBS - 1);
            let row = j * (cols + 1);
            let mut n = next(open, row + (si0 >> SHIFT));
            while n <= row + (si1 >> SHIFT) {
                'cell: {
                    let c = n - j;
                    let i = c - j * cols;
                    // For a cell wholly in the face, the blocks under its part of the rectangle:
                    // if none is solid, its sub-blocks are not looked at; if all are, they
                    // are not tested one by one, and if the rectangle holds the cell, it is
                    // covered at once.
                    let [cl, cr, cb, ct] = cell_tangents[c].map(|v| v * distance);
                    let mut solid = false;
                    if !cl.is_nan() {
                        let (some, all) = under(cl.max(left), cr.min(right), cb.max(bottom), ct.min(top));
                        if some == 0 {
                            break 'cell;
                        }
                        solid = some == all;
                        if solid && cl >= left && cr <= right && cb >= bottom && ct <= top {
                            bits[c] = FULL;
                            covered[c] = true;
                            open[n] = n + 1;
                            break 'cell;
                        }
                    }
                    // The cell's sub-blocks in the box not covered yet, row by row.
                    let si_from = si0.max(i * SUBS);
                    let si_to = si1.min(i * SUBS + SUBS - 1);
                    let mut todo = (rows_mask(sj_from & MASK, sj_to & MASK) * columns_mask(si_from & MASK, si_to & MASK)) & !bits[c];
                    while todo != 0 {
                        let bit = todo.trailing_zeros() as usize;
                        todo &= todo - 1;
                        let s = sub_block(c, bit);
                        // The sub-block's part of the plane is within this rectangle.
                        let [l, r, b, t] = tangents[s].map(|v| v * distance);
                        let (l, r, b, t) = (l.max(left), r.min(right), b.max(bottom), t.min(top));
                        if l >= r || b >= t || !(solid || all_solid(l, r, b, t)) {
                            continue;
                        }
                        if area[s] == 0.0 {
                            touched.push(s);
                        }
                        area[s] += (r - l) * (t - b);
                    }
                }
                n = next(open, n + 1);
            }
        }
    }

    /** Ends a layer at `distance` begun with cover(): sub-blocks are covered if its opaque pieces add up to the whole of them. */
    pub fn end_layer(&mut self, distance: f64) {
        let Some(face) = self.face else { return };
        let Coverage { cols, ref shapes, ref mut bits, ref mut covered, ref mut open, ref mut area, ref mut touched, .. } = *self;
        let Some(Shape { tangents, .. }) = &shapes[face] else { return };
        for &s in touched.iter() {
            let [l, r, b, t] = tangents[s];
            let whole = (r - l) * (t - b) * distance * distance;
            if area[s] >= whole * (1.0 - 1e-9) {
                let c = s / (SUBS * SUBS);
                bits[c] |= 1 << (s % (SUBS * SUBS));
                if bits[c] == FULL {
                    covered[c] = true;
                    let n = c + c / cols;
                    open[n] = n + 1;
                }
            }
            area[s] = 0.0;
        }
        touched.clear();
    }

    /** Marks chunk `handle` used in this render; false if it is not known (see add). */
    pub fn use_chunk(&mut self, handle: u32) -> bool {
        self.chunks.get_mut(&handle).map(|chunk| chunk.used = self.renders).is_some()
    }

    /** Adds chunk `handle`, used in this render (see Chunk::new). */
    pub fn add(&mut self, handle: u32, width: usize, height: usize, scale: f64, blocks: &[u8]) {
        let chunk = Chunk { used: self.renders, ..Chunk::new(width, height, scale, blocks) };
        self.chunks.insert(handle, chunk);
    }
}

// The WebAssembly interface, for occlusion.ts.

thread_local! {
    static COVERAGE: RefCell<Coverage> = RefCell::new(Coverage::default());
    /** A chunk's blocks being handed over: see coverage_blocks. */
    static BLOCKS: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/** Starts a render (see Coverage::set_view). */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_view(width: f64, height: f64, focal: f64) {
    COVERAGE.with_borrow_mut(|coverage| coverage.set_view(width, height, focal));
}

/** Starts a face (see Coverage::set_face): the camera's right, up and forward in its coordinates. */
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn coverage_face(face: u32, rx: f64, ry: f64, rz: f64, ux: f64, uy: f64, uz: f64, fx: f64, fy: f64, fz: f64) {
    COVERAGE.with_borrow_mut(|coverage| coverage.set_face(face as usize, [rx, ry, rz], [ux, uy, uz], [fx, fy, fz]));
}

/** See Coverage::hides. */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_hides(min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> u32 {
    COVERAGE.with_borrow(|coverage| coverage.hides(min_x, max_x, min_y, max_y) as u32)
}

/** See Coverage::use_chunk: 1 if chunk `handle` is known, else 0, and it is to be handed over with coverage_blocks and coverage_add. */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_use(handle: u32) -> u32 {
    COVERAGE.with_borrow_mut(|coverage| coverage.use_chunk(handle) as u32)
}

/** Where to write the `count` blocks of a chunk to add. */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_blocks(count: u32) -> *mut u8 {
    BLOCKS.with_borrow_mut(|blocks| {
        blocks.clear();
        blocks.resize(count as usize, 0);
        blocks.as_mut_ptr()
    })
}

/** Adds a chunk, its blocks written to coverage_blocks (see Coverage::add). */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_add(handle: u32, width: u32, height: u32, scale: f64) {
    BLOCKS.with_borrow(|blocks| COVERAGE.with_borrow_mut(|coverage| coverage.add(handle, width as usize, height as usize, scale, blocks)));
}

/** See Coverage::hides_chunk. */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_hides_chunk(handle: u32, x: f64, y: f64, distance: f64) -> u32 {
    COVERAGE.with_borrow(|coverage| coverage.hides_chunk(handle, x, y, distance) as u32)
}

/** See Coverage::cover. */
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn coverage_cover(
    handle: u32,
    x: f64,
    y: f64,
    cell_left: f64,
    cell_bottom: f64,
    cell_width: f64,
    cell_height: f64,
    distance: f64,
) {
    COVERAGE.with_borrow_mut(|coverage| coverage.cover(handle, x, y, cell_left, cell_bottom, cell_width, cell_height, distance));
}

/** See Coverage::end_layer. */
#[unsafe(no_mangle)]
pub extern "C" fn coverage_end_layer(distance: f64) {
    COVERAGE.with_borrow_mut(|coverage| coverage.end_layer(distance));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_bounds_solid_blocks_as_scanning_them_does() {
        // 23 × 14 pixels: 6 × 4 blocks, the last column and row cut by the edge.
        #[rustfmt::skip]
        let solid = [
            0, 0, 0, 0, 0, 0,
            0, 1, 1, 0, 0, 1,
            0, 0, 1, 1, 0, 0,
            0, 1, 0, 1, 1, 1,
        ];
        let chunk = Chunk::new(23, 14, 1.0, &solid.map(|s| s * (SOLID | DRAWN)));
        for x0 in 0..=6 {
            for x1 in x0..=6 {
                for y0 in 0..=4 {
                    for y1 in y0..=4 {
                        let scanned = (y0..y1).flat_map(|y| (x0..x1).map(move |x| usize::from(solid[y * 6 + x]))).sum();
                        assert_eq!(chunk.solid_in(x0, x1, y0, y1), scanned);
                    }
                }
            }
        }
        // Left, right, top, bottom in pixels: past the empty first row and column, to the cut ones.
        assert_eq!(chunk.solid, [4.0, 23.0, 4.0, 14.0]);
    }

    #[test]
    fn shapes_a_face_as_looking_at_each_sub_block_does() {
        let (width, height, focal) = (333.0, 217.0, 180.0);
        let mut coverage = Coverage::default();
        coverage.set_view(width, height, focal);
        let Coverage { cols, rows, .. } = coverage;
        let tangent_bounds = |rays: &[Vec3]| {
            let mut b = Bounds::EMPTY;
            rays.iter().for_each(|r| b.add(r[0] / r[2], r[1] / r[2]));
            [b.min_x, b.max_x, b.min_y, b.max_y]
        };
        let (mut inside, mut outside, mut crossing) = (0, 0, 0);
        let mut old = None;
        // Turned every way, reusing the previous shape's buffers.
        for (yaw, pitch) in [(0.0f64, 0.0f64), (0.7, 0.3), (2.0, -0.9), (-2.8, 1.2), (1.3, 0.05), (-0.5, -0.6)] {
            let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
            let look = Look { width, height, focal, right: [cy, 0.0, -sy], up: [-sy * sp, cp, -cy * sp], forward: [sy * cp, sp, cy * cp] };
            let shape = coverage.shape(look, old.take());
            for c in 0..cols * rows {
                let (i, j) = (c % cols, c / cols);
                let mut bits = 0u16;
                for bit in 0..SUBS * SUBS {
                    let (si, sj) = (i * SUBS + bit % SUBS, j * SUBS + bit / SUBS);
                    let rays =
                        [(si, sj), (si + 1, sj), (si + 1, sj + 1), (si, sj + 1)].map(|(a, b)| look.ray((a * SUB) as f64, (b * SUB) as f64));
                    let codes = rays.map(sides);
                    let tangents = if codes == [0; 4] {
                        Some(tangent_bounds(&rays))
                    } else if codes.iter().fold(15, |all, &o| all & o) == 0 {
                        tangent_box(&rays).map(|b| [b.min_x, b.max_x, b.min_y, b.max_y])
                    } else {
                        None
                    };
                    match tangents {
                        Some(t) => assert_eq!(shape.tangents[sub_block(c, bit)], t),
                        None => bits |= 1 << bit,
                    }
                }
                assert_eq!(shape.outside[c], bits);
                if bits == FULL {
                    outside += 1;
                    continue;
                }
                // Wholly inside when its corners are; else NaN, not to be covered at once.
                let rays = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)].map(|(a, b)| look.ray((a * CELL) as f64, (b * CELL) as f64));
                if rays.map(sides) == [0; 4] {
                    assert_eq!(shape.cell_tangents[c], tangent_bounds(&rays));
                    inside += 1;
                } else {
                    assert!(shape.cell_tangents[c].iter().all(|v| v.is_nan()));
                    crossing += 1;
                }
            }
            old = Some(shape);
        }
        assert!(inside > 100 && outside > 100 && crossing > 100, "{inside} {outside} {crossing}");
    }
}
