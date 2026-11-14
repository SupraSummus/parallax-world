//! Draws a frame's chunks with wgpu, as compositor.ts does with WebGL 2,
//! into a picture of its own, which the window (or a shot) shows:
//! cells' reliefs of whole voxels, a quad per tile of their columns whose
//! pixels' rays find the column they meet first, or (Frame's `relief` off)
//! their whole voxels as splats, each at its depth; only the tiles or
//! clusters of splats that may show, culled on the GPU against the depth of
//! what the last frame showed (compositor.wgsl), then the sky behind them.
//! See-through voxels come after, as pictures of each cell's medium slab
//! by slab, far to near, each over those behind, then over the picture.
//!
//! A chunk's columns and splats are uploaded when it comes into view, into
//! a buffer each for all, in clusters (tiles of columns, and splats near
//! each other on its plane), with its record (where it is, wherever the
//! camera, and which clusters are its), and dropped when it leaves, as the
//! frame's changes say (renderer.rs's Change): a change costs what changed,
//! its writes one copy that the GPU scatters (Arena). Each frame writes
//! only the globals: the GPU places the chunks relative to the camera
//! (`place`), and culls the clusters, all there are, that its eye does not
//! draw.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Range;

use bake::Medium;
use bake::lod::Lod;
use bake::slicing::{LAYER, half_layer};
use egui_wgpu::wgpu;
use fcc::light::{linear_to_srgb, srgb_to_linear};
use fcc::{FastMap, FastSet};

use super::renderer::{Frame, Placement};
use crate::engine::{Colours, Look, Painter};
use crate::gpu::Easel;
use crate::view::{self, Vec3, to_local, view_basis};

/**
 * A chunk as it is, wherever the camera (Record in compositor.wgsl), which
 * `place` makes the chunk the shaders read (Chunk) of each frame, from the
 * camera: written once, when the chunk comes. Its face's coordinates, in
 * world units.
 */
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuRecord {
    /** Top-left corner of what it draws. */
    origin: [f32; 2],
    /** Its plane's depth along the axis: its cell's site's. */
    depth: f32,
    face: u32,
    /** Its cell's level, and its grid's, and its site's c in its grid's units (CellChunk). */
    level: i32,
    atoms: i32,
    site: i32,
    /** RGBA to paint its voxels, shaded as they are (Colours); 0 for their own colours. */
    tint: u32,
    /** Drawn flat (Flat): the box its columns fill, from its origin; its plane past its own, z0, gx, gy, and how far its edge pillars reach from it. */
    min: [f32; 2],
    max: [f32; 2],
    plane: [f32; 4],
    /** Its clusters of whole voxels, [first, end) of all, its medium's from there to `veils`; none if its slot is free. */
    whole: [u32; 2],
    /** A half layer of its grid: the unit of its columns' and splats' depths. */
    half_layer: f32,
    veils: u32,
}

/** Bytes of a chunk's record, and of a chunk as the shaders read it, from the camera (Chunk in compositor.wgsl). */
pub const RECORD: u64 = size_of::<GpuRecord>() as u64;
const CHUNK: u64 = 80;

/**
 * A column of a chunk's relief (columns in compositor.wgsl): its front's
 * depth and its run, in half layers, as i16s, and its colour; 0, none. Drawn
 * flat, a run of its picture or two colours instead (flatten).
 */
pub type GpuColumn = [u32; 2];

/** A splat (splat_vertex's instance in compositor.wgsl): its numbers as a cell's are baked (bake's bake_cell). */
pub type GpuSplat = [u32; 5];

/**
 * A cluster (Cluster in compositor.wgsl): a tile of a chunk's columns, or
 * splats of it, or runs of a picture. Where they are along the axis,
 * relative to the chunk's origin, and how far past the plane, nearest and
 * farthest; a tile's (or picture's) first column's centre, and the
 * splats' widest's half width; its first column or splat, its chunk's
 * slot and how many splats or runs, and whether it showed in the last
 * frame; of a picture of medium, its slab's first layer, and how many half
 * layers its halves are (bake's Medium).
 */
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuCluster {
    min: [f32; 2],
    max: [f32; 2],
    base: [f32; 2],
    first: u32,
    slot_count: u32,
    visible: u32,
    near: f32,
    far: f32,
    wide: f32,
    layer: f32,
    half: f32,
}

impl GpuCluster {
    fn new(first: usize, count: usize, base: [f64; 2]) -> GpuCluster {
        GpuCluster {
            min: [f32::INFINITY; 2],
            max: [f32::NEG_INFINITY; 2],
            base: base.map(|v| v as f32),
            first: first as u32,
            slot_count: count as u32,
            visible: 0,
            near: f32::INFINITY,
            far: f32::NEG_INFINITY,
            wide: 0.0,
            layer: 0.0,
            half: 0.0,
        }
    }

    /** Holds a box from (x, y) half `w` wide, `h` high either way, `near` to `far` past the plane. */
    fn hold(&mut self, [x, y]: [f64; 2], [w, h]: [f64; 2], [near, far]: [f64; 2]) {
        self.min = [self.min[0].min((x - w) as f32), self.min[1].min((y - h) as f32)];
        self.max = [self.max[0].max((x + w) as f32), self.max[1].max((y + h) as f32)];
        (self.near, self.far) = (self.near.min(near as f32), self.far.max(far as f32));
    }
}

/** Columns a tile has across and rows of them (a column's row is its b, and a row's columns are 2 apart in a). */
const TILE: i32 = 4;

/**
 * A chunk's columns (bake's Baker::columns) as the GPU takes them, in
 * tiles of TILE × TILE, nearest first, so that what is nearer is mostly
 * drawn first; their `first` counts from the chunk's first column, and
 * their chunk's slot is not in them yet. `base` is where its column (0, 0)
 * is, relative to its origin, `unit` its grid's spacing, both in world
 * units, and `half_layer` a half layer of it. In the bake workers, so that
 * a chunk's upload is a copy.
 */
pub fn tiles(baked: &[[u32; 3]], base: [f64; 2], unit: f64, half_layer: f64) -> (Vec<GpuColumn>, Vec<GpuCluster>) {
    let i16s = |v: u32| [v as u16, (v >> 16) as u16].map(|v| i32::from(v as i16));
    // A column (a, b) is in row b, a's of its row 2 apart.
    let place = |[a, b]: [i32; 2]| [(a - (b & 1)) / 2, b];
    let tile_of = |c: &[u32; 3]| place(i16s(c[0])).map(|v| v.div_euclid(TILE));
    let mut sorted: Vec<&[u32; 3]> = baked.iter().collect();
    sorted.sort_by_key(|c| tile_of(c));
    let mut columns = Vec::new();
    let mut tiles = Vec::new();
    for tile in sorted.chunk_by(|p, q| tile_of(p) == tile_of(q)) {
        let [i, j] = tile_of(tile[0]);
        let first = columns.len();
        columns.resize(first + (TILE * TILE) as usize, [0, 0]);
        let centre = |[a, b]: [i32; 2]| [base[0] + a as f64 * LAYER * unit, base[1] + b as f64 * unit];
        // Its first column, (2 TILE i, TILE j), even.
        let mut t = GpuCluster::new(first, 0, centre([2 * TILE * i, TILE * j]));
        for c in tile {
            let ab = i16s(c[0]);
            let [s, r] = place(ab);
            columns[first + (r.rem_euclid(TILE) * TILE + s.rem_euclid(TILE)) as usize] = [c[1], c[2]];
            let [depth, run] = i16s(c[1]);
            // Its cell (column_at in compositor.wgsl): |u| ≤ LAYER, LAYER |u| + |v| ≤ 3/4, in units.
            t.hold(centre(ab), [LAYER * unit, 0.75 * unit], [depth, depth + run].map(|d| d as f64 * half_layer));
        }
        tiles.push(t);
    }
    tiles.sort_by(|a, b| a.near.total_cmp(&b.near));
    (columns, tiles)
}

/**
 * A chunk's relief drawn flat (an experiment): a picture of its columns'
 * colours, without their depths, on a plane fixed in the world, which the
 * camera's perspective moves and scales.
 * A column's pillar is what the relief shows of it: a ray slanting σ off
 * the axis sees it over σ times its length across. On the plane a column
 * shows at one depth instead, so where two cells' pictures meet, on planes
 * of their own, a gap may open, at most σ times how far the edge columns'
 * pillars (those beside an empty column) reach from their planes: along a
 * surface parallel to the axis most, where a picture is a row or two of
 * columns and their pillars run through the cell.
 * So the plane is the one they reach least from (`flatten`), and each
 * frame the picture is drawn enlarged, each edge out by that reach times
 * the slant of the rays to it (`place` in compositor.wgsl), at most the face's
 * (`MAX_SLANT`): neighbours overlap where their pillars would, and depth
 * decides between them.
 */
#[derive(Clone, Copy, Default)]
pub struct Flat {
    /** Its plane: past the chunk's plane by z0 + gx x + gy y at (x, y) from its origin, all in world units. */
    pub plane: [f64; 3],
    /** How far along the axis its edge columns' pillars reach from the plane, at most. */
    pub reach: f64,
    /** The box its columns' bricks fill, from its origin. */
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/** The face's rays' tangents across and up, at most (|x| + √2 |y| ≤ z). */
const MAX_SLANT: [f64; 2] = [1.0, std::f64::consts::FRAC_1_SQRT_2];

/** Runs of a picture a cluster holds at most (its count is a byte). */
const RUNS: usize = 255;

/**
 * A chunk's relief drawn flat (Flat), from its columns as tiles() takes
 * them, as the GPU takes it. Its picture is rows of columns, a column
 * (a, b) in row b, a row's columns 2 apart in a, each a brick a row high
 * and 2 LAYER wide, so that the rows, half a brick apart, fill the plane;
 * it is drawn as the runs of its rows' columns (empty ones, colour 0, left
 * out), a quad each, so that its pixels need nothing left out:
 * runs, each [its row, first column and length (packed: flat_vertex),
 * its first colour], then the colours, two to an item; its first column
 * the least row's (even, so that rows alternate as from (0, 0)) least
 * column in a row. Runs by rows in clusters of at most RUNS, each holding
 * its runs' box, as wide as the picture may be enlarged, and as far past
 * the plane as the picture's plane is over it.
 * Of planes of slope g, the one the edge pillars reach least from runs
 * midway through the thinnest slab of that slope holding them, and they
 * reach half its thickness from it: a convex function of g, least within
 * the slopes no steeper than the face's rays (so that none goes along the
 * plane but at the face's edge), found by golden sections.
 */
pub fn flatten(baked: &[[u32; 3]], base: [f64; 2], unit: f64, half_layer: f64) -> (Vec<GpuColumn>, Vec<GpuCluster>, Flat) {
    let i16s = |v: u32| [v as u16, (v >> 16) as u16].map(|v| i32::from(v as i16));
    // Each column's place (a, b), centre, and its pillar's ends past the chunk's plane.
    let columns: Vec<([i32; 2], [f64; 2], [f64; 2])> = baked
        .iter()
        .map(|c| {
            let [a, b] = i16s(c[0]);
            let [depth, run] = i16s(c[1]);
            ([a, b], [base[0] + a as f64 * LAYER * unit, base[1] + b as f64 * unit], [depth, depth + run].map(|d| d as f64 * half_layer))
        })
        .collect();
    if columns.is_empty() {
        return (Vec::new(), Vec::new(), Flat::default());
    }
    // The edges: a, b of one parity, six neighbours.
    let places: FastSet<[i32; 2]> = columns.iter().map(|c| c.0).collect();
    let edges: Vec<([f64; 2], [f64; 2])> = columns
        .iter()
        .filter(|(ab, ..)| {
            [[2, 0], [-2, 0], [1, 1], [-1, 1], [1, -1], [-1, -1]].iter().any(|d| !places.contains(&[ab[0] + d[0], ab[1] + d[1]]))
        })
        .map(|&(_, centre, ends)| (centre, ends))
        .collect();
    // The slab of slope g holding the edge pillars: its middle at (0, 0), and half its thickness.
    let slab = |[gx, gy]: [f64; 2]| {
        let (lo, hi) = edges.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), ([x, y], [front, back])| {
            let s = gx * x + gy * y;
            (lo.min(front - s), hi.max(back - s))
        });
        ((lo + hi) / 2.0, (hi - lo) / 2.0)
    };
    let [most_x, most_y] = MAX_SLANT.map(|s| 1.0 / s);
    let gy_for = |gx: f64| least(|gy| slab([gx, gy]).1, most_y);
    let gx = least(|gx| slab([gx, gy_for(gx)]).1, most_x);
    let g = [gx, gy_for(gx)];
    let (z0, reach) = slab(g);
    // A brick's half width and height.
    let brick = [LAYER * unit, 0.5 * unit];
    let box_of = |centres: &mut dyn Iterator<Item = [f64; 2]>| {
        centres.fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(min, max), c| {
            ([0, 1].map(|d| min[d].min(c[d] - brick[d])), [0, 1].map(|d| max[d].max(c[d] + brick[d])))
        })
    };
    let (min, max) = box_of(&mut columns.iter().map(|c| c.1));
    let flat = Flat { plane: [z0, g[0], g[1]], reach, min, max };
    // Its picture: a column's place (r, s), from the first.
    let mut least = columns.iter().fold([i32::MAX; 2], |l, c| [0, 1].map(|d| l[d].min(place(c.0)[d])));
    least[0] &= !1;
    let mut shown: Vec<([u32; 2], u32)> = baked
        .iter()
        .zip(&columns)
        .filter(|(c, _)| c[2] != 0)
        .map(|(c, (ab, ..))| (std::array::from_fn(|d| (place(*ab)[d] - least[d]) as u32), c[2]))
        .collect();
    shown.sort_unstable_by_key(|c| c.0);
    let runs = runs(&mut shown.iter().map(|c| c.0));
    // Its first colour's, in halves of items, from its cluster's first run.
    let mut picture = packed(&runs, |k, colours| 2 * runs.len() + colours - 2 * (k - k % RUNS));
    picture.extend(shown.chunks(2).map(|pair| [pair[0].1, pair.get(1).map_or(0, |p| p.1)]));
    let first = column_centre(base, unit, least, 0, 0);
    let margin = MAX_SLANT.map(|s| s * reach);
    let clusters = runs
        .chunks(RUNS)
        .enumerate()
        .map(|(k, part)| {
            let (lo, hi) = runs_box(part, base, unit, least);
            let depths = corners(lo, hi).map(|q| flat.depth(q));
            let mut cluster = GpuCluster::new(k * RUNS, part.len(), first);
            let half = [0, 1].map(|d| (hi[d] - lo[d]) / 2.0 + margin[d]);
            let near_far = [depths.into_iter().fold(f64::INFINITY, f64::min), depths.into_iter().fold(f64::NEG_INFINITY, f64::max)];
            cluster.hold([0, 1].map(|d| (lo[d] + hi[d]) / 2.0), half, near_far);
            cluster
        })
        .collect();
    (picture, clusters, flat)
}

/** The runs [row, first, length] of a picture's places (r, s), sorted. */
fn runs(places: &mut dyn Iterator<Item = [u32; 2]>) -> Vec<[u32; 3]> {
    let mut runs: Vec<[u32; 3]> = Vec::new();
    for [r, s] in places {
        match runs.last_mut() {
            Some(run) if run[0] == r && run[1] + run[2] == s => run[2] += 1,
            _ => runs.push([r, s, 1]),
        }
    }
    runs
}

/** Runs as the GPU takes them (flat_vertex, veil_vertex): packed, and where `colours(k, columns before)` says the k-th's colours are. */
fn packed(runs: &[[u32; 3]], colours: impl Fn(usize, usize) -> usize) -> Vec<GpuColumn> {
    let mut before = 0;
    runs.iter()
        .enumerate()
        .map(|(k, &[r, s, n])| {
            assert!(r < 1 << 10 && s < 1 << 11 && n < 1 << 11, "a picture of {} rows", r + 1);
            let run = [r | s << 10 | n << 21, colours(k, before) as u32];
            before += n as usize;
            run
        })
        .collect()
}

/** The centre of column s of row r of a picture whose least place is `least` (flatten). */
fn column_centre(base: [f64; 2], unit: f64, least: [i32; 2], r: u32, s: u32) -> [f64; 2] {
    let first = [base[0] + (2 * least[1]) as f64 * LAYER * unit, base[1] + least[0] as f64 * unit];
    [first[0] + f64::from(2 * s + (r & 1)) * LAYER * unit, first[1] + f64::from(r) * unit]
}

/** The box runs' bricks fill. */
fn runs_box(runs: &[[u32; 3]], base: [f64; 2], unit: f64, least: [i32; 2]) -> ([f64; 2], [f64; 2]) {
    let brick = [LAYER * unit, 0.5 * unit];
    runs.iter()
        .flat_map(|&[r, s, n]| [column_centre(base, unit, least, r, s), column_centre(base, unit, least, r, s + n - 1)])
        .fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(min, max), c| {
            ([0, 1].map(|d| min[d].min(c[d] - brick[d])), [0, 1].map(|d| max[d].max(c[d] + brick[d])))
        })
}

/** A picture's place (r, s) of column (a, b): its row, and its count in it. */
fn place([a, b]: [i32; 2]) -> [i32; 2] {
    [b, (a - (b & 1)).div_euclid(2)]
}

/**
 * A chunk's medium (bake's Medium) as the GPU takes it, `start` columns
 * into the chunk's: a picture of it per slab, on the
 * slab's plane, of its own columns' bricks as flatten lays them out, drawn
 * as its rows' runs, and a column around them each way, its neighbours';
 * each picture its columns, all in its box from its first place, row by
 * row, each an item: [colour, its optical depths in its slab's nearer half
 * and its farther, as f16s], 0 where none; then its runs
 * of its own columns, each [its row, first column and length (packed, as
 * flatten's), how many items back its first column is | how many columns
 * its rows are << 20]. Its runs in clusters of at most RUNS, each holding
 * its runs' box, its picture's plane, its slab's first layer and its
 * halves' depth.
 * `base`, `unit` and `half_layer` as tiles() takes them. However the
 * cells share out a slab, its columns tile its plane once.
 */
pub fn veils(medium: &[Medium], base: [f64; 2], unit: f64, half_layer: f64, slab: i32, start: usize) -> (Vec<GpuColumn>, Vec<GpuCluster>) {
    let (mut items, mut clusters) = (Vec::new(), Vec::new());
    for picture in medium.chunk_by(|p, q| p.first == q.first) {
        let own = || picture.iter().filter(|p| p.own).map(|p| place(p.column));
        let (Some(low), Some(high)) =
            (own().reduce(|l, p| [l[0].min(p[0]), l[1].min(p[1])]), own().reduce(|h, p| [h[0].max(p[0]), h[1].max(p[1])]))
        else {
            continue;
        };
        // A row and a column around its own, its first row even.
        let least = [(low[0] - 1) & !1, low[1] - 1];
        let [rows, width] = [high[0] - least[0] + 2, high[1] - least[1] + 2];
        let at = |p: [i32; 2]| [p[0] - least[0], p[1] - least[1]];
        let first = items.len();
        items.resize(first + (rows * width) as usize, [0; 2]);
        for p in picture {
            let [r, s] = at(place(p.column));
            if (0..rows).contains(&r) && (0..width).contains(&s) {
                let [near, far] = p.depth.map(f16_bits);
                items[first + (r * width + s) as usize] = [p.colour, near | far << 16];
            }
        }
        let runs = runs(&mut own().map(|p| at(p).map(|v| v as u32)));
        let runs_first = items.len();
        assert!(width < 1 << 12 && runs_first + runs.len() - first < 1 << 20, "a picture of {width} columns");
        items.extend(packed(&runs, |k, _| (runs_first + k - first) | (width as usize) << 20));
        // Its sites' outlines span a layer either side of them.
        let depth = f64::from(picture[0].first + slab - 1) * half_layer;
        clusters.extend(runs.chunks(RUNS).enumerate().map(|(k, part)| {
            let (lo, hi) = runs_box(part, base, unit, least);
            let mut cluster = GpuCluster::new(start + runs_first + k * RUNS, part.len(), column_centre(base, unit, least, 0, 0));
            cluster.hold([0, 1].map(|d| (lo[d] + hi[d]) / 2.0), [0, 1].map(|d| (hi[d] - lo[d]) / 2.0), [depth; 2]);
            (cluster.layer, cluster.half) = (picture[0].first as f32, slab as f32);
            cluster
        }));
    }
    (items, clusters)
}

/**
 * The bits of the f16 nearest `v`, as WGSL's unpack2x16float takes them:
 * of an optical depth, at least a voxel's of alpha 1/255 (worldgen's),
 * well above f16's least normal, and at most a few dozen; 0, none.
 */
fn f16_bits(v: f32) -> u32 {
    if v <= 0.0 {
        return 0;
    }
    let bits = v.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    assert!((1..31).contains(&exponent), "an optical depth of {v}");
    // Rounded to nearest: a carry into the exponent is right.
    let mantissa = (bits & 0x7f_ffff) + 0x1000;
    ((exponent as u32) << 10) + (mantissa >> 13)
}

/** The corners of a box. */
fn corners(min: [f64; 2], max: [f64; 2]) -> [[f64; 2]; 4] {
    [min, [max[0], min[1]], [min[0], max[1]], max]
}

impl Flat {
    /** How far its plane is past its chunk's at (x, y) from its origin. */
    fn depth(&self, [x, y]: [f64; 2]) -> f64 {
        let [z0, gx, gy] = self.plane;
        z0 + gx * x + gy * y
    }
}

/** Where in [−most, most] the convex `f` is least, by golden sections, to a ten-thousandth of `most`. */
fn least(f: impl Fn(f64) -> f64, most: f64) -> f64 {
    let r = (5f64.sqrt() - 1.0) / 2.0;
    let (mut lo, mut hi) = (-most, most);
    let (mut a, mut b) = (hi - r * (hi - lo), lo + r * (hi - lo));
    let (mut fa, mut fb) = (f(a), f(b));
    while hi - lo > 1e-4 * most {
        if fa <= fb {
            (hi, b, fb) = (b, a, fa);
            a = hi - r * (hi - lo);
            fa = f(a);
        } else {
            (lo, a, fa) = (a, b, fb);
            b = lo + r * (hi - lo);
            fb = f(b);
        }
    }
    (lo + hi) / 2.0
}

/**
 * Splats at most in a cluster (its count is a byte), and points across the
 * blocks of a chunk's plane its clusters gather splats from, where the chunk
 * is when it comes into view.
 */
const CLUSTER: usize = 16;
const BLOCK: f64 = 12.0;

/** The size of the blocks that cluster() gathers splats from, for a chunk on a plane `distance` ahead seen at `focal`. */
pub fn block(distance: f64, focal: f64) -> f64 {
    BLOCK * distance / focal
}

/**
 * A chunk's splats as the GPU takes them, its whole voxels' where they are
 * no relief, in clusters: of the splats in each block of size `block` on
 * its plane, nearest first in each, so that a block's first clusters are
 * its front, the clusters nearest first, so that splats are mostly drawn
 * near to far. Their `first` counts from the chunk's first splat, and
 * their chunk's slot is not in them yet.
 * A cluster's box holds its splats' outlines along the axis, from `near`
 * to `far` past the plane, `half_layer` per half layer of their depth, and
 * how wide the widest is: the GPU finds where they are drawn for its
 * camera. In the bake workers, so that a chunk's upload is a copy.
 */
pub fn cluster(numbers: &[f32], half_layer: f64, block: f64) -> (Vec<GpuSplat>, Vec<GpuCluster>) {
    let numbers = numbers.as_chunks::<5>().0;
    let block_of = |s: usize| [1, 0].map(|d| (numbers[s][d] as f64 / block).floor() as i64);
    let depth = |s: usize| numbers[s][3] as i32;
    let mut sorted: Vec<usize> = (0..numbers.len()).collect();
    sorted.sort_by_key(|&s| (block_of(s), depth(s)));
    let of = |part: &[usize], first: usize| {
        let mut cluster = GpuCluster::new(first, part.len(), [0.0; 2]);
        for &s in part {
            let [x, y, w, depth, ..] = numbers[s].map(f64::from);
            cluster.hold([x, y], [w, w * std::f64::consts::SQRT_2], [depth * half_layer; 2]);
            cluster.wide = cluster.wide.max(w as f32);
        }
        cluster
    };
    let mut clusters = Vec::new();
    let mut start = 0;
    for run in sorted.chunk_by(|&a, &b| block_of(a) == block_of(b)) {
        // Split evenly.
        let parts = run.len().div_ceil(CLUSTER);
        for p in 0..parts {
            let first = run.len() * p / parts;
            clusters.push(of(&run[first..run.len() * (p + 1) / parts], start + first));
        }
        start += run.len();
    }
    clusters.sort_by(|a, b| a.near.total_cmp(&b.near));
    (sorted.iter().map(|&s| numbers[s].map(f32::to_bits)).collect(), clusters)
}

/** A chunk's columns, splats and clusters on the GPU, and its slot in the table (its record's place). */
struct Resident {
    columns: Range<u32>,
    splats: Range<u32>,
    clusters: Range<u32>,
    slot: u32,
    /** Its record, and its tint for each of Colours, by Colours::ALL. */
    record: GpuRecord,
    tints: [u32; 3],
    /** Whether the frame no longer draws it: it is to be dropped. */
    going: bool,
}

/**
 * Chunks a frame uploads or drops at most when it follows the frame's
 * changes: a face picked anew brings thousands, tens of milliseconds of
 * the main thread's at once. Those that came go first.
 */
const CHANGES: usize = 1024;

/**
 * A buffer of items `stride` bytes long, ranges of them allocated and
 * freed, and written: the writes gathered until staged, their words one
 * after another and where each run of them goes, which the GPU scatters,
 * so that any number of places cost a copy and a dispatch.
 */
struct Arena {
    label: &'static str,
    stride: u64,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    /** Items the buffer holds, the end of those in use, and the free ranges below it. */
    capacity: u32,
    top: u32,
    free: Free,
    /** Bytes to write, by the item they start at: a later write there replaces an earlier. */
    writes: BTreeMap<u32, Vec<u8>>,
    /** The words of the writes staged last, and their runs (`scatter` in compositor.wgsl). */
    words: wgpu::Buffer,
    runs: wgpu::Buffer,
}

impl Arena {
    fn new(device: &wgpu::Device, label: &'static str, stride: u64, usage: wgpu::BufferUsages, capacity: u32) -> Arena {
        let usage = usage | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let capacity = capacity.min(Arena::most(device, stride, usage));
        let buffer = buffer(device, label, capacity as u64 * stride, usage);
        let (words, runs) = (storage(device, label, 1 << 16), storage(device, label, 1 << 12));
        Arena { label, stride, usage, buffer, capacity, top: 0, free: Free::default(), writes: BTreeMap::new(), words, runs }
    }

    /** Writes `bytes` from item `at` on, once staged. */
    fn write(&mut self, at: u32, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.writes.insert(at, bytes.to_vec());
        }
    }

    /**
     * Stages the writes on `belt`, their words one after another, and their
     * runs, each [where it goes, its first word, how many words], after how
     * many runs and words (`scatter` in compositor.wgsl, with `pipelines`);
     * all to be done as `encoder` runs.
     */
    fn stage(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        pipelines: &Pipelines,
    ) {
        if self.writes.is_empty() {
            return;
        }
        let mut words: Vec<u8> = Vec::new();
        let mut runs: Vec<[u32; 4]> = vec![[0; 4]];
        for (at, bytes) in std::mem::take(&mut self.writes) {
            let (to, from, count) = ((u64::from(at) * self.stride / 4) as u32, (words.len() / 4) as u32, (bytes.len() / 4) as u32);
            // Where the last run ends, it goes on.
            match runs.last_mut().filter(|run| run[0] + run[2] == to && run[2] > 0) {
                Some(run) => run[2] += count,
                None => runs.push([to, from, count, 0]),
            }
            words.extend(bytes);
        }
        let count = (words.len() / 4) as u32;
        runs[0] = [runs.len() as u32 - 1, count, 0, 0];
        at_least(device, &mut self.words, words.len() as u64);
        at_least(device, &mut self.runs, runs.len() as u64 * 16);
        stage(belt, encoder, &self.words, 0, &words);
        stage(belt, encoder, &self.runs, 0, bytemuck::cast_slice(&runs));
        let entries = [(19, self.buffer.as_entire_binding()), (20, self.words.as_entire_binding()), (21, self.runs.as_entire_binding())];
        let group = bind(device, &pipelines.scatter, &entries);
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipelines.scatter_words);
        pass.set_bind_group(0, &group, &[]);
        let [across, rows] = groups(count.div_ceil(CULL_THREADS));
        pass.dispatch_workgroups(across, rows, 1);
    }

    /** The most items of `stride` bytes a buffer of `usage` may hold on the device. */
    fn most(device: &wgpu::Device, stride: u64, usage: wgpu::BufferUsages) -> u32 {
        let limits = device.limits();
        let mut most = limits.max_buffer_size;
        if usage.contains(wgpu::BufferUsages::STORAGE) {
            most = most.min(limits.max_storage_buffer_binding_size);
        }
        (most / stride).min(u32::MAX as u64) as u32
    }

    /** Grown to `capacity` items, as far as the device allows, what it holds copied over (writes not staged yet go to the new). */
    fn grow(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, capacity: u32) {
        let capacity = capacity.min(Arena::most(device, self.stride, self.usage));
        if capacity > self.capacity {
            let grown = buffer(device, self.label, capacity as u64 * self.stride, self.usage);
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &grown, 0, self.top as u64 * self.stride);
            (self.buffer, self.capacity) = (grown, capacity);
        }
    }

    /** `count` items' room in the buffer, grown if need be; none when the device allows no more. */
    fn allocate(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, count: u32) -> Option<Range<u32>> {
        if count == 0 {
            return Some(0..0);
        }
        if let Some(r) = self.free.take(count) {
            return Some(r);
        }
        if self.top + count > self.capacity {
            self.grow(device, encoder, (self.capacity * 2).max(self.top + count));
        }
        (self.top + count <= self.capacity).then(|| {
            self.top += count;
            self.top - count..self.top
        })
    }

    /** Frees a range, merging it with free neighbours; what was to be written there is not. */
    fn release(&mut self, r: Range<u32>) {
        if r.is_empty() {
            return;
        }
        self.writes.remove(&r.start);
        let r = self.free.give(r);
        if r.end == self.top {
            self.free.remove(r.clone());
            self.top = r.start;
        }
    }
}

/**
 * Free ranges, none touching another: by start, and by length, so that
 * taking the least that fits and giving one back, merged with its
 * neighbours, cost a logarithm of how many there are (thousands).
 */
#[derive(Default)]
struct Free {
    /** By start, each's end. */
    ends: BTreeMap<u32, u32>,
    /** Each's (length, start). */
    lengths: BTreeSet<(u32, u32)>,
}

impl Free {
    /** `count` items of the least range that holds them, if any does. */
    fn take(&mut self, count: u32) -> Option<Range<u32>> {
        let &(_, start) = self.lengths.range((count, 0)..).next()?;
        let end = self.ends[&start];
        self.remove(start..end);
        if end - start > count {
            self.add(start + count..end);
        }
        Some(start..start + count)
    }

    /** Adds `r`, merged with the free ranges either side: the range it is now part of. */
    fn give(&mut self, mut r: Range<u32>) -> Range<u32> {
        if let Some(end) = self.ends.get(&r.end).copied() {
            self.remove(r.end..end);
            r.end = end;
        }
        if let Some((&start, &end)) = self.ends.range(..r.start).next_back()
            && end == r.start
        {
            self.remove(start..end);
            r.start = start;
        }
        self.add(r.clone());
        r
    }

    fn add(&mut self, r: Range<u32>) {
        self.ends.insert(r.start, r.end);
        self.lengths.insert((r.end - r.start, r.start));
    }

    /** Removes free range `r`. */
    fn remove(&mut self, r: Range<u32>) {
        self.ends.remove(&r.start);
        self.lengths.remove(&(r.end - r.start, r.start));
    }
}

/** What the compositor draws into, the size of the view in pixels. */
struct Targets {
    size: [u32; 2],
    picture: wgpu::Texture,
    depth: wgpu::Texture,
    /** The pictures of medium, one over another: their colour as much as they stop, and how much (veil_fragment). */
    veil: wgpu::Texture,
    composite: wgpu::BindGroup,
    /** The pyramid of farthest depths, and per level a bind group to make it and its size. */
    farthest: wgpu::Texture,
    levels: Vec<(wgpu::BindGroup, [u32; 2])>,
    show: wgpu::BindGroup,
}

/** The bind group layouts and pipelines (compositor.wgsl). */
struct Pipelines {
    draw: wgpu::BindGroupLayout,
    cull: wgpu::BindGroupLayout,
    reduce_depth: wgpu::BindGroupLayout,
    reduce: wgpu::BindGroupLayout,
    show: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
    place: wgpu::BindGroupLayout,
    scatter: wgpu::BindGroupLayout,
    /** The tiles of whole voxels, or their splats; then the pictures of medium one over another, and those over the picture. */
    tiles: wgpu::RenderPipeline,
    flat: wgpu::RenderPipeline,
    splats: wgpu::RenderPipeline,
    veil: wgpu::RenderPipeline,
    composite_picture: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    seams: wgpu::RenderPipeline,
    show_picture: wgpu::RenderPipeline,
    cull_drawn: wgpu::ComputePipeline,
    cull_rest: wgpu::ComputePipeline,
    scan_veils: wgpu::ComputePipeline,
    order_veils: wgpu::ComputePipeline,
    reduce_depth_level: wgpu::ComputePipeline,
    reduce_level: wgpu::ComputePipeline,
    place_chunks: wgpu::ComputePipeline,
    scatter_words: wgpu::ComputePipeline,
}

pub struct Compositor {
    format: wgpu::TextureFormat,
    pipelines: Pipelines,
    globals: wgpu::Buffer,
    /** By slot, the chunks' records, and the chunks `place` makes of them each frame. */
    records: Arena,
    table: wgpu::Buffer,
    columns: Arena,
    splats: Arena,
    clusters: Arena,
    resident: FastMap<u32, Resident>,
    /**
     * What it writes into its buffers but the globals, staged in buffers it
     * keeps mapped, copied as the frame's commands run: a chunk's upload a
     * copy into memory, not a write of its own (Queue::write_buffer makes
     * a buffer for each).
     */
    belt: wgpu::util::StagingBelt,
    /** The number of the frame prepared last, and the colours: until they change, a frame is its globals. */
    prepared: Option<(u64, Colours)>,
    /**
     * The frame's chunks to upload yet, in its order, and to drop (those
     * resident that it no longer draws), as CHANGES and the buffers' room
     * allow.
     */
    coming: VecDeque<Placement>,
    going: Vec<u32>,
    /** Whether the frame prepared last left some of them to later frames, CHANGES taken. */
    held_back: bool,
    /** How many clusters of pictures of medium the chunks it has hold. */
    veils: u32,
    /** By phase, the draws of clusters the culling lists, and how many; then the pictures of medium's (VEILS). */
    draws: [wgpu::Buffer; 3],
    drawn: [wgpu::Buffer; 3],
    /** How many pictures of medium the culling counts at each place along the rays, then where the next of each is listed (order_veils). */
    veil_places: wgpu::Buffer,
    targets: Option<Targets>,
    seams: bool,
    /** Whether the frame's whole voxels are reliefs, or splats (Frame's `relief`), and its reliefs flat (Frame's `flat`). */
    relief: bool,
    flat: bool,
    /** What draws, and by phase what culls. */
    draw_group: Option<wgpu::BindGroup>,
    cull_groups: Vec<wgpu::BindGroup>,
    place_group: Option<wgpu::BindGroup>,
}

const SKY_TOP: [f32; 3] = [0x3f as f32 / 255.0, 0x73 as f32 / 255.0, 0xb8 as f32 / 255.0];
const SKY_HORIZON: [f32; 3] = [0xb9 as f32 / 255.0, 0xd3 as f32 / 255.0, 0xe8 as f32 / 255.0];
/** f32 in Globals (compositor.wgsl), and where its `whole` is. */
const GLOBALS: usize = 276;
const WHOLE: usize = 6;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const FARTHEST: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const VEIL: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/** Where in `draws` and `drawn` the pictures of medium's are. */
const VEILS: usize = 2;

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    buffer(device, label, size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST)
}

/** Bytes of each of the belt's buffers: a few frames of chunks coming into view. */
const BELT: u64 = 1 << 21;

/** Stages `bytes` on `belt` to be written into `target` at `offset` as `encoder` runs. */
fn stage(belt: &mut wgpu::util::StagingBelt, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
    if let Some(size) = wgpu::BufferSize::new(bytes.len() as u64) {
        belt.write_buffer(encoder, target, offset, size).copy_from_slice(bytes);
    }
}

/** Threads a workgroup that culls has, a cluster each (compositor.wgsl). */
const CULL_THREADS: u32 = 64;

/**
 * Workgroups for `n`: in as many rows as it takes of at most as many as a
 * dispatch may have along an axis.
 */
fn groups(n: u32) -> [u32; 2] {
    let most = wgpu::Limits::default().max_compute_workgroups_per_dimension;
    let rows = n.div_ceil(most).max(1);
    [n.div_ceil(rows), rows]
}

/** Replaces `buffer` if it holds fewer than `size` bytes, by one of the next power of two. */
fn at_least(device: &wgpu::Device, buffer: &mut wgpu::Buffer, size: u64) {
    if buffer.size() < size {
        *buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            // Room to grow, as far as the device allows.
            size: size.next_power_of_two().min(device.limits().max_buffer_size).max(size),
            usage: buffer.usage(),
            mapped_at_creation: false,
        });
    }
}

fn layout(device: &wgpu::Device, entries: &[(u32, wgpu::ShaderStages, wgpu::BindingType)]) -> wgpu::BindGroupLayout {
    let entries: Vec<_> =
        entries.iter().map(|&(binding, visibility, ty)| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None }).collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries })
}

fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, entries: &[(u32, wgpu::BindingResource)]) -> wgpu::BindGroup {
    let entries: Vec<_> =
        entries.iter().map(|(binding, resource)| wgpu::BindGroupEntry { binding: *binding, resource: resource.clone() }).collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries: &entries })
}

impl Pipelines {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, culls: bool) -> Pipelines {
        use wgpu::ShaderStages as S;
        let shader = device.create_shader_module(wgpu::include_wgsl!("compositor.wgsl"));
        let buffer = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let uniform = buffer(wgpu::BufferBindingType::Uniform);
        let read = buffer(wgpu::BufferBindingType::Storage { read_only: true });
        let write = buffer(wgpu::BufferBindingType::Storage { read_only: false });
        let texture =
            |sample_type| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let float = texture(wgpu::TextureSampleType::Float { filterable: false });
        let level_out = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: FARTHEST,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let draw = layout(
            device,
            &[
                (0, S::VERTEX_FRAGMENT, uniform),
                (1, S::VERTEX_FRAGMENT, read),
                (2, S::VERTEX_FRAGMENT, read),
                (3, S::VERTEX_FRAGMENT, read),
            ],
        );
        let cull = layout(
            device,
            &[
                (0, S::COMPUTE, uniform),
                (1, S::COMPUTE, read),
                (4, S::COMPUTE, write),
                (6, S::COMPUTE, write),
                (7, S::COMPUTE, write),
                (8, S::COMPUTE, float),
                (9, S::COMPUTE, write),
                (16, S::COMPUTE, write),
                (22, S::COMPUTE, write),
            ],
        );
        let reduce_depth = layout(device, &[(10, S::COMPUTE, texture(wgpu::TextureSampleType::Depth)), (12, S::COMPUTE, level_out)]);
        let reduce = layout(device, &[(11, S::COMPUTE, float), (12, S::COMPUTE, level_out)]);
        let show = layout(device, &[(13, S::FRAGMENT, float)]);
        let composite = layout(device, &[(14, S::FRAGMENT, float)]);
        let place = layout(device, &[(0, S::COMPUTE, uniform), (17, S::COMPUTE, read), (18, S::COMPUTE, write)]);
        let scatter = layout(device, &[(19, S::COMPUTE, write), (20, S::COMPUTE, read), (21, S::COMPUTE, read)]);
        let pipeline_layout = |layout: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            })
        };
        let splat = wgpu::VertexBufferLayout {
            array_stride: size_of::<GpuSplat>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4],
        };
        // Ties (the sky, cleared to 1) are drawn.
        let depth = |compare, write| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let (tested, over) = (Some(depth(wgpu::CompareFunction::LessEqual, true)), Some(depth(wgpu::CompareFunction::Always, false)));
        let behind = Some(depth(wgpu::CompareFunction::LessEqual, false));
        let target = |format, blend| Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL });
        let blend = |src_factor, dst_factor| wgpu::BlendComponent { src_factor, dst_factor, operation: wgpu::BlendOperation::Add };
        use wgpu::BlendFactor as F;
        // Over what is drawn before, as much as it stops: its colour premultiplied by that.
        let over_veils = wgpu::BlendState { color: blend(F::One, F::OneMinusSrcAlpha), alpha: blend(F::One, F::OneMinusSrcAlpha) };
        // Over the picture: their colour, and the picture as much as they let through.
        let over_picture = wgpu::BlendState { color: blend(F::One, F::SrcAlpha), alpha: blend(F::Zero, F::One) };
        let render = |layout: &wgpu::BindGroupLayout,
                      vertex: &str,
                      fragment: &str,
                      buffers: &[Option<wgpu::VertexBufferLayout>],
                      depth_stencil: Option<wgpu::DepthStencilState>,
                      targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout(layout)),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let compute = |layout: &wgpu::BindGroupLayout, entry: &str, constants: &[(&str, f64)]| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout(layout)),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
                cache: None,
            })
        };
        Pipelines {
            tiles: render(&draw, "tile_vertex", "tile_fragment", &[], tested.clone(), &[target(format, None)]),
            flat: render(&draw, "flat_vertex", "flat_fragment", &[], tested.clone(), &[target(format, None)]),
            splats: render(&draw, "splat_vertex", "splat_fragment", &[Some(splat)], tested.clone(), &[target(format, None)]),
            veil: render(&draw, "veil_vertex", "veil_fragment", &[], behind, &[target(VEIL, Some(over_veils))]),
            composite_picture: render(
                &composite,
                "sky_vertex",
                "composite_fragment",
                &[],
                over.clone(),
                &[target(format, Some(over_picture))],
            ),
            sky: render(&draw, "sky_vertex", "sky_fragment", &[], tested, &[target(format, None)]),
            seams: render(&draw, "sky_vertex", "seams_fragment", &[], over, &[target(format, Some(wgpu::BlendState::ALPHA_BLENDING))]),
            show_picture: render(&show, "show_vertex", "show_fragment", &[], None, &[target(format, None)]),
            cull_drawn: compute(&cull, "cull_drawn", &[]),
            cull_rest: compute(&cull, "cull_rest", &[("culls", f64::from(culls))]),
            scan_veils: compute(&cull, "scan_veils", &[]),
            order_veils: compute(&cull, "order_veils", &[("culls", f64::from(culls))]),
            reduce_depth_level: compute(&reduce_depth, "reduce_depth", &[]),
            reduce_level: compute(&reduce, "reduce", &[]),
            place_chunks: compute(&place, "place", &[]),
            scatter_words: compute(&scatter, "scatter", &[]),
            draw,
            cull,
            reduce_depth,
            reduce,
            show,
            composite,
            place,
            scatter,
        }
    }
}

impl Targets {
    fn new(device: &wgpu::Device, pipelines: &Pipelines, format: wgpu::TextureFormat, [width, height]: [u32; 2]) -> Targets {
        let texture = |format, usage, mip_level_count, width, height| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as U;
        let picture = texture(format, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC, 1, width, height);
        let depth = texture(DEPTH, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING, 1, width, height);
        // Level 0 covers the view at 2 × 2 pixels a texel, in powers of two (so each next level halves the last), down to a texel.
        let mut sizes = vec![[width.div_ceil(2).next_power_of_two(), height.div_ceil(2).next_power_of_two()]];
        while let Some(&[w, h]) = sizes.last().filter(|s| s[0] > 1 || s[1] > 1) {
            sizes.push([(w / 2).max(1), (h / 2).max(1)]);
        }
        let farthest = texture(FARTHEST, U::STORAGE_BINDING | U::TEXTURE_BINDING, sizes.len() as u32, sizes[0][0], sizes[0][1]);
        let level = |k: usize| {
            farthest.create_view(&wgpu::TextureViewDescriptor { base_mip_level: k as u32, mip_level_count: Some(1), ..Default::default() })
        };
        let levels = sizes
            .iter()
            .enumerate()
            .map(|(k, &size)| {
                let group = if k == 0 {
                    let depth = depth.create_view(&Default::default());
                    bind(
                        device,
                        &pipelines.reduce_depth,
                        &[(10, wgpu::BindingResource::TextureView(&depth)), (12, wgpu::BindingResource::TextureView(&level(0)))],
                    )
                } else {
                    bind(
                        device,
                        &pipelines.reduce,
                        &[(11, wgpu::BindingResource::TextureView(&level(k - 1))), (12, wgpu::BindingResource::TextureView(&level(k)))],
                    )
                };
                (group, size)
            })
            .collect();
        let show = bind(device, &pipelines.show, &[(13, wgpu::BindingResource::TextureView(&picture.create_view(&Default::default())))]);
        let veil = texture(VEIL, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING, 1, width, height);
        let composite =
            bind(device, &pipelines.composite, &[(14, wgpu::BindingResource::TextureView(&veil.create_view(&Default::default())))]);
        Targets { size: [width, height], picture, depth, veil, composite, farthest, levels, show }
    }
}

impl Compositor {
    /** For a target of `format`; `culls` off, it draws every cluster in view (compositor.wgsl). */
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, culls: bool) -> Compositor {
        use wgpu::BufferUsages as U;
        let draws = |label| buffer(device, label, (1 << 20).min(device.limits().max_buffer_size), U::STORAGE | U::INDIRECT | U::COPY_SRC);
        let drawn = |label| buffer(device, label, 4, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        Compositor {
            format,
            pipelines: Pipelines::new(device, format, culls),
            globals: buffer(device, "globals", (GLOBALS * 4) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            records: Arena::new(device, "records", RECORD, U::STORAGE, 1024),
            table: buffer(device, "chunks", 1024 * CHUNK, U::STORAGE),
            columns: Arena::new(device, "columns", size_of::<GpuColumn>() as u64, wgpu::BufferUsages::STORAGE, 1 << 20),
            splats: Arena::new(device, "splats", size_of::<GpuSplat>() as u64, wgpu::BufferUsages::VERTEX, 1 << 20),
            clusters: Arena::new(device, "clusters", size_of::<GpuCluster>() as u64, wgpu::BufferUsages::STORAGE, 1 << 15),
            resident: FastMap::default(),
            belt: wgpu::util::StagingBelt::new(device.clone(), BELT),
            prepared: None,
            coming: VecDeque::new(),
            held_back: false,
            going: Vec::new(),
            veils: 0,
            draws: [draws("draws first"), draws("draws next"), draws("veils")],
            drawn: [drawn("drawn first"), drawn("drawn next"), drawn("veils drawn")],
            veil_places: buffer(device, "veil places", 4, U::STORAGE | U::COPY_DST),
            targets: None,
            seams: false,
            relief: true,
            flat: false,
            draw_group: None,
            cull_groups: Vec::new(),
            place_group: None,
        }
    }

    /** How many tiles (or whole splats) the frame drawn last drew in each phase, as `easel` reads them from the GPU. */
    pub fn drawn(&self, easel: &Easel) -> [u32; 2] {
        std::array::from_fn(|phase| {
            let n = u32::from_le_bytes(easel.read_buffer(&self.drawn[phase], 4).try_into().expect("count")) as u64;
            let draws = easel.read_buffer(&self.draws[phase], (n * 16).max(16));
            draws.as_chunks::<16>().0[..n as usize].iter().map(|d| u32::from_le_bytes(d[4..8].try_into().expect("draw"))).sum()
        })
    }

    /** Uploads `p`'s chunk's columns, splats and clusters (see tiles and cluster), and its record, unless the buffers can hold no more. */
    fn upload(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, p: &Placement, colours: Colours) -> bool {
        let baked = &p.chunk.baked;
        let mut arenas = [&mut self.records, &mut self.columns, &mut self.splats, &mut self.clusters];
        let counts = [1, baked.columns.len(), baked.splats.len(), baked.clusters.len()];
        let mut ranges: [Range<u32>; 4] = Default::default();
        for k in 0..4 {
            let Some(range) = arenas[k].allocate(device, encoder, counts[k] as u32) else {
                (0..k).for_each(|j| arenas[j].release(ranges[j].clone()));
                return false;
            };
            ranges[k] = range;
        }
        let [slot, columns, splats, range] = ranges;
        let slot = slot.start;
        self.columns.write(columns.start, bytemuck::cast_slice(&baked.columns));
        self.splats.write(splats.start, bytemuck::cast_slice(&baked.splats));
        let clusters: Vec<GpuCluster> = baked
            .clusters
            .iter()
            .enumerate()
            .map(|(k, c)| {
                // Its whole voxels' first: tiles of columns, if the frame's are reliefs; its medium's runs.
                let first = if k < baked.veils && !self.relief { splats.start } else { columns.start };
                GpuCluster { first: first + c.first, slot_count: slot << 8 | c.slot_count, ..*c }
            })
            .collect();
        self.clusters.write(range.start, bytemuck::cast_slice(&clusters));
        // A hue per layer of a level's cells (Colours), neighbours in a stack far apart.
        let cell = &p.cell;
        let hue = hash(&[p.face as u64, cell.level as u64]) + 0.618 * (cell.site[2] / cell.spacing()) as f64;
        let tints = [0, tint(hue), tint(hash(&[hue.to_bits(), cell.site[0] as u64, cell.site[1] as u64]))];
        let flat = &baked.flat;
        let veils = range.start + baked.veils as u32;
        let record = GpuRecord {
            origin: baked.origin.map(|v| v as f32),
            depth: (f64::from(cell.site[2] << cell.atoms) * LAYER) as f32,
            face: p.face as u32,
            level: cell.level,
            atoms: cell.atoms,
            site: cell.site[2],
            tint: tints[colours as usize],
            min: flat.min.map(|v| v as f32),
            max: flat.max.map(|v| v as f32),
            plane: [flat.plane[0], flat.plane[1], flat.plane[2], flat.reach].map(|v| v as f32),
            whole: [range.start, veils],
            half_layer: half_layer(cell.atoms) as f32,
            veils: range.end,
        };
        self.records.write(slot, bytemuck::bytes_of(&record));
        self.veils += range.end - veils;
        self.resident.insert(p.chunk.id, Resident { columns, splats, clusters: range, slot, record, tints, going: false });
        true
    }

    /** Drops chunk `id`'s columns, splats and clusters, if it has them, and its record: its slot's clusters are none. */
    fn release(&mut self, id: u32) {
        let Some(r) = self.resident.remove(&id) else { return };
        self.columns.release(r.columns);
        self.splats.release(r.splats);
        self.clusters.release(r.clusters);
        self.records.release(r.slot..r.slot + 1);
        self.records.write(r.slot, bytemuck::bytes_of(&GpuRecord::default()));
        self.veils -= r.record.veils - r.record.whole[1];
    }

    /**
     * Uploads what `frame` draws that is not on the GPU yet, and drops what
     * it no longer draws, once its chunks or the colours change: as its
     * changes since the frame prepared last say, if it keeps them, CHANGES
     * a frame, else taking it whole at once; and every frame, writes the
     * globals.
     */
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &Frame, view: &View) {
        let size = view.pixels.map(|v| v.max(1));
        (self.seams, self.relief, self.flat) = (view.seams, frame.relief, frame.flat);
        let resized = self.targets.as_ref().is_none_or(|t| t.size != size);
        if resized {
            self.targets = Some(Targets::new(device, &self.pipelines, self.format, size));
        }
        let same = self.prepared == Some((frame.number, view.colours));
        if !same {
            match self.prepared.filter(|p| p.1 == view.colours).and_then(|p| frame.since(p.0)) {
                Some(changes) => changes.iter().for_each(|c| self.follow(&c.went, &c.came)),
                None => self.rebuild(device, encoder, frame, view.colours),
            }
            self.prepared = Some((frame.number, view.colours));
        }
        let pending = !self.coming.is_empty() || !self.going.is_empty();
        self.held_back = pending && self.apply(device, encoder, view.colours, CHANGES);
        if !same || pending {
            self.flush(device, encoder);
        }
        let places = 2 * 4 * view.medium_halves();
        let grown = self.veil_places.size() < places;
        at_least(device, &mut self.veil_places, places);
        if !same || pending || resized || grown {
            self.bind_groups(device);
        }
        let sky_fog = 1.0 - (-frame.farthest / view.fog_distance).exp();
        let mut globals = view.globals(size, sky_fog);
        globals[WHOLE] = if frame.flat {
            2.0
        } else if frame.relief {
            0.0
        } else {
            1.0
        };
        queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));
    }

    /** A change of the frame's chunks, to apply: those that `came`, after what is still to come, and those that `went`. */
    fn follow(&mut self, went: &[u32], came: &[Placement]) {
        let mut gone = FastSet::default();
        for &id in went {
            match self.resident.get_mut(&id) {
                Some(r) => {
                    r.going = true;
                    self.going.push(id);
                }
                None => _ = gone.insert(id),
            }
        }
        if !gone.is_empty() {
            self.coming.retain(|p| !gone.contains(&p.chunk.id));
        }
        for p in came {
            // Back before it was dropped, it stays.
            match self.resident.get_mut(&p.chunk.id) {
                Some(r) => r.going = false,
                None => self.coming.push_back(p.clone()),
            }
        }
    }

    /**
     * Uploads what is to come and drops what is to go, `most` of them: what
     * comes first, in the frame's order (finest and nearest first), as far
     * as there is room, dropping what goes to make some; what is farther
     * waits. Whether `most` left some to a later frame.
     */
    fn apply(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, colours: Colours, mut most: usize) -> bool {
        while most > 0 {
            if let Some(p) = self.coming.front().cloned()
                && self.upload(device, encoder, &p, colours)
            {
                self.coming.pop_front();
            } else if let Some(id) = self.going.pop() {
                if !self.resident.get(&id).is_some_and(|r| r.going) {
                    continue;
                }
                self.release(id);
            } else {
                break;
            }
            most -= 1;
        }
        most == 0 && (!self.coming.is_empty() || !self.going.is_empty())
    }

    /** Takes `frame` whole, painted in `colours`: drops what it does not draw, and uploads what it lacks. */
    fn rebuild(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, frame: &Frame, colours: Colours) {
        if self.prepared.is_some_and(|p| p.1 != colours) {
            for r in self.resident.values_mut() {
                r.record.tint = r.tints[colours as usize];
                self.records.write(r.slot, bytemuck::bytes_of(&r.record));
            }
        }
        let ids: FastSet<u32> = frame.chunks.iter().map(|p| p.chunk.id).collect();
        let went: Vec<u32> = self.resident.keys().filter(|id| !ids.contains(id)).copied().collect();
        self.coming.clear();
        self.going.clear();
        self.resident.values_mut().for_each(|r| r.going = false);
        let came: Vec<Placement> = frame.chunks.iter().filter(|p| !self.resident.contains_key(&p.chunk.id)).cloned().collect();
        self.follow(&went, &came);
        self.apply(device, encoder, colours, usize::MAX);
    }

    /** Stages what the changes wrote; makes room for what the GPU makes of them. */
    fn flush(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        for arena in [&mut self.records, &mut self.columns, &mut self.splats, &mut self.clusters] {
            arena.stage(device, encoder, &mut self.belt, &self.pipelines);
        }
        at_least(device, &mut self.table, u64::from(self.records.capacity) * CHUNK);
        for draws in &mut self.draws {
            at_least(device, draws, u64::from(self.clusters.top.max(1)) * 16);
        }
        // Its writes copied over as the encoder runs.
        self.belt.finish_and_recall_on_submit(encoder);
    }

    fn bind_groups(&mut self, device: &wgpu::Device) {
        let t = self.targets.as_ref().expect("targets");
        let farthest = t.farthest.create_view(&Default::default());
        use wgpu::BindingResource as R;
        let entries = [
            (0, self.globals.as_entire_binding()),
            (1, self.table.as_entire_binding()),
            (2, self.clusters.buffer.as_entire_binding()),
            (3, self.columns.buffer.as_entire_binding()),
        ];
        self.draw_group = Some(bind(device, &self.pipelines.draw, &entries));
        self.cull_groups = (0..2)
            .map(|k| {
                let entries = [
                    (0, self.globals.as_entire_binding()),
                    (1, self.table.as_entire_binding()),
                    (4, self.clusters.buffer.as_entire_binding()),
                    (6, self.draws[k].as_entire_binding()),
                    (7, self.drawn[k].as_entire_binding()),
                    (8, R::TextureView(&farthest)),
                    (9, self.drawn[VEILS].as_entire_binding()),
                    (16, self.draws[VEILS].as_entire_binding()),
                    (22, self.veil_places.as_entire_binding()),
                ];
                bind(device, &self.pipelines.cull, &entries)
            })
            .collect();
        let entries =
            [(0, self.globals.as_entire_binding()), (17, self.records.buffer.as_entire_binding()), (18, self.table.as_entire_binding())];
        self.place_group = Some(bind(device, &self.pipelines.place, &entries));
    }

    /**
     * Draws the frame prepared last into the picture: its chunks placed
     * from the camera (place in compositor.wgsl); the tiles (or
     * clusters of whole splats) that showed last frame, then those that show now that did not, culled
     * against the farthest depths of the first; then the pictures of
     * medium in front of them, far to near, and those over the picture.
     */
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder) {
        let t = self.targets.as_ref().expect("prepared");
        let p = &self.pipelines;
        for drawn in &self.drawn {
            encoder.clear_buffer(drawn, 0, None);
        }
        encoder.clear_buffer(&self.veil_places, 0, None);
        // Every slot, those freed too, so that the clusters left where their chunks were are none of theirs.
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&p.place_chunks);
            pass.set_bind_group(0, self.place_group.as_ref(), &[]);
            let [across, rows] = groups(self.records.capacity.div_ceil(CULL_THREADS));
            pass.dispatch_workgroups(across, rows, 1);
        }
        let cull = |encoder: &mut wgpu::CommandEncoder, phase: usize| {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            if phase == 1 {
                // The pyramid of what the first phase drew.
                for (k, (group, [width, height])) in t.levels.iter().enumerate() {
                    pass.set_pipeline(if k == 0 { &p.reduce_depth_level } else { &p.reduce_level });
                    pass.set_bind_group(0, group, &[]);
                    pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
                }
            }
            let clusters = self.clusters.top;
            if clusters > 0 {
                pass.set_pipeline(if phase == 0 { &p.cull_drawn } else { &p.cull_rest });
                pass.set_bind_group(0, &self.cull_groups[phase], &[]);
                let [across, rows] = groups(clusters.div_ceil(CULL_THREADS));
                pass.dispatch_workgroups(across, rows, 1);
                if phase == 1 {
                    pass.set_pipeline(&p.scan_veils);
                    pass.dispatch_workgroups(1, 1, 1);
                    pass.set_pipeline(&p.order_veils);
                    pass.dispatch_workgroups(across, rows, 1);
                }
            }
        };
        let tiles = |pass: &mut wgpu::RenderPass<'_>, phase: usize| {
            pass.set_bind_group(0, self.draw_group.as_ref(), &[]);
            if self.flat {
                pass.set_pipeline(&p.flat);
            } else if self.relief {
                pass.set_pipeline(&p.tiles);
            } else {
                pass.set_pipeline(&p.splats);
                pass.set_vertex_buffer(0, self.splats.buffer.slice(..));
            }
            pass.multi_draw_indirect_count(&self.draws[phase], 0, &self.drawn[phase], 0, self.clusters.top);
        };
        cull(encoder, 0);
        tiles(&mut pass(encoder, &[(&t.picture, Some(wgpu::Color::BLACK))], (&t.depth, Some(1.0))), 0);
        cull(encoder, 1);
        {
            let mut pass = pass(encoder, &[(&t.picture, None)], (&t.depth, None));
            tiles(&mut pass, 1);
            pass.set_pipeline(&p.sky);
            pass.draw(0..4, 0..1);
        }
        if self.veils > 0 {
            // None yet.
            let mut veil = pass(encoder, &[(&t.veil, Some(wgpu::Color::TRANSPARENT))], (&t.depth, None));
            veil.set_bind_group(0, self.draw_group.as_ref(), &[]);
            veil.set_pipeline(&p.veil);
            veil.multi_draw_indirect_count(&self.draws[VEILS], 0, &self.drawn[VEILS], 0, self.clusters.top);
        }
        let mut pass = pass(encoder, &[(&t.picture, None)], (&t.depth, None));
        if self.veils > 0 {
            pass.set_pipeline(&p.composite_picture);
            pass.set_bind_group(0, &t.composite, &[]);
            pass.draw(0..4, 0..1);
        }
        if self.seams {
            pass.set_pipeline(&p.seams);
            pass.set_bind_group(0, self.draw_group.as_ref(), &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

/** A render pass into `colours`, each cleared to its colour or else kept, tested against `depth`, cleared to its value or else kept. */
fn pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    colours: &[(&wgpu::Texture, Option<wgpu::Color>)],
    depth: (&wgpu::Texture, Option<f32>),
) -> wgpu::RenderPass<'e> {
    let views: Vec<_> = colours.iter().map(|(texture, _)| texture.create_view(&Default::default())).collect();
    let attachments: Vec<_> = views
        .iter()
        .zip(colours)
        .map(|(view, (_, clear))| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store },
            })
        })
        .collect();
    let depth_view = depth.0.create_view(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &attachments,
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth_view,
            depth_ops: Some(wgpu::Operations {
                load: depth.1.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/** How a frame is seen: orientation, focal length, fog, the faces' axes, and the view's size in points and in pixels (the picture's). */
#[derive(Clone, Copy)]
pub struct View {
    pub yaw: f64,
    pub pitch: f64,
    pub focal: f64,
    pub fog_distance: f64,
    pub faces: [[Vec3; 3]; 12],
    pub size: [f32; 2],
    pub pixels: [u32; 2],
    /** Whether to draw the seams between faces over the picture. */
    pub seams: bool,
    pub colours: Colours,
    /** How bright the sky is: 1 by day, less as --light dims it. */
    pub sky: f32,
    pub camera: Vec3,
    /** Which cells each level claims (Lod::layers), and which the medium's, by their Lods (Lod::for_medium): each its Q, layers of grid 0 to the render distance, its coarsest level and its finest. */
    pub lod: [f32; 4],
    pub medium: [f32; 4],
}

/** A number in [0, 1) from `numbers`, as if at random. */
fn hash(numbers: &[u64]) -> f64 {
    let h = numbers.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &n| (h ^ n).wrapping_mul(0x0100_0000_01b3).rotate_left(29));
    (h.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 11) as f64 / (1u64 << 53) as f64
}

/** A bright colour of `hue` (in turns), as RGBA. */
fn tint(hue: f64) -> u32 {
    let channel = |n: f64| {
        let k = (n + hue.rem_euclid(1.0) * 6.0) % 6.0;
        (255.0 * (1.0 - 0.75 * k.min(4.0 - k).clamp(0.0, 1.0))) as u32
    };
    0xff00_0000 | channel(1.0) << 16 | channel(3.0) << 8 | channel(5.0)
}

impl View {
    /** As `look` sees, at its size in pixels. */
    pub fn new(look: &Look) -> View {
        let faces = view::faces();
        let whole = Lod::new(look.shift, look.detail);
        let lod = |lod: Lod| [lod.q as f32, (look.far / LAYER).ceil() as f32, lod.top() as f32, lod.finest() as f32];
        View {
            yaw: look.yaw,
            pitch: look.pitch,
            focal: look.view().focal,
            fog_distance: look.far * look.fog,
            faces: std::array::from_fn(|f| faces[f].axes),
            size: look.size.map(|v| v as f32),
            pixels: look.size.map(|v| v as u32),
            seams: look.seams,
            colours: look.colours,
            sky: look.light.map_or(1.0, |b| b.sky),
            camera: look.camera,
            lod: lod(whole),
            medium: lod(whole.for_medium()),
        }
    }

    /**
     * How many half slabs of medium a ray of a face may cross (veil_place):
     * of every level but the coarsest, as many as fit between its planes,
     * 2Q + 1 at most, or the finest's, 4Q + 1, and two more; of the
     * coarsest, to a cell past the render distance, a slab past that.
     */
    fn medium_halves(&self) -> u64 {
        let [q, layers, top, finest] = self.medium.map(f64::from);
        ((top - finest) * (4.0 * q + 3.0) + layers / 2f64.powf(top - 1.0) + 6.0).ceil() as u64
    }

    fn globals(&self, pixels: [u32; 2], sky_fog: f64) -> [f32; GLOBALS] {
        let mut g = [0.0f32; GLOBALS];
        g[0..2].copy_from_slice(&self.size);
        g[2..4].copy_from_slice(&pixels.map(|v| v as f32));
        g[4] = self.focal as f32;
        g[5] = self.fog_distance as f32;
        g[7] = sky_fog as f32;
        // Dimmed in linear terms, as light adds up.
        let dim = |colour: [f32; 3]| colour.map(|c| linear_to_srgb(srgb_to_linear(c) * self.sky));
        g[8..11].copy_from_slice(&dim(SKY_TOP));
        g[12..15].copy_from_slice(&dim(SKY_HORIZON));
        let axes = view_basis(self.yaw, self.pitch);
        let columns = |g: &mut [f32], axes: [Vec3; 3]| {
            for (c, axis) in axes.iter().enumerate() {
                for d in 0..3 {
                    g[c * 4 + d] = axis[d] as f32;
                }
            }
        };
        columns(&mut g[16..28], axes);
        for (f, face) in self.faces.iter().enumerate() {
            let at = 28 + f * 20;
            columns(&mut g[at..at + 12], axes.map(|axis| to_local(face, axis)));
            g[at + 12..at + 15].copy_from_slice(&face[2].map(|v| (v * std::f64::consts::SQRT_2).round() as f32));
            g[at + 16..at + 19].copy_from_slice(&to_local(face, self.camera).map(|v| v as f32));
        }
        g[268..272].copy_from_slice(&self.lod);
        g[272..276].copy_from_slice(&self.medium);
        g
    }
}

impl Painter for Compositor {
    fn picture(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("a frame").picture
    }

    fn settled(&self) -> bool {
        !self.held_back
    }

    fn show(&self, pass: &mut wgpu::RenderPass<'_>) {
        let t = self.targets.as_ref().expect("prepared");
        pass.set_pipeline(&self.pipelines.show_picture);
        pass.set_bind_group(0, &t.show, &[]);
        pass.draw(0..4, 0..1);
    }

    fn describe(&self, easel: &Easel) -> String {
        let [first, next] = self.drawn(easel);
        let what = if self.flat {
            "runs of flat pictures' columns"
        } else if self.relief {
            "tiles of columns"
        } else {
            "whole splats"
        };
        format!(": {what}, {first} that showed before and {next} more")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_ranges_merge_and_give_the_least_that_fits() {
        let mut free = Free::default();
        for r in [0..4, 10..12, 20..30, 4..6] {
            free.give(r);
        }
        assert_eq!(free.ends.iter().map(|(&s, &e)| s..e).collect::<Vec<_>>(), [0..6, 10..12, 20..30]);
        assert_eq!(free.take(2), Some(10..12));
        assert_eq!(free.take(3), Some(0..3));
        assert_eq!(free.take(11), None);
        assert_eq!(free.give(6..20), 3..30);
        assert_eq!(free.take(27), Some(3..30));
        assert!(free.ends.is_empty() && free.lengths.is_empty());
    }

    #[test]
    fn groups_hold_every_chunk_in_rows_a_dispatch_allows() {
        let most = wgpu::Limits::default().max_compute_workgroups_per_dimension;
        for n in [0, 1, 1000, most, most + 1, 3 * most + 7] {
            let [across, rows] = groups(n);
            // Within the limit, all of them, and at most a workgroup a row to spare.
            assert!(across <= most && across * rows >= n && across * rows < n + rows, "{n}: {across} × {rows}");
        }
    }
}
