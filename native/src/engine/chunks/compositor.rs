//! Draws a frame's chunks with wgpu, as compositor.ts does with WebGL 2,
//! into a picture of its own, which the window (or a shot) shows:
//! cells' reliefs of whole voxels, a quad per tile of their columns whose
//! pixels' rays find the column they meet first, or (Frame's `relief` off)
//! their whole voxels as splats, each at its depth; only the tiles or
//! clusters of splats that may show, culled on the GPU against the depth of
//! what the last frame showed (compositor.wgsl), then the sky behind them.
//! See-through voxels come after, as splats, nearest first, each over what
//! is behind it as much as it stops rays, then over the picture.
//!
//! A chunk's columns and splats are uploaded when it comes into view, into
//! a buffer each for all, in clusters (tiles of columns, and splats near
//! each other on its plane), and dropped when it leaves; each
//! frame rewrites only a table of the chunks' placements, relative to the
//! camera, so precision does not fall off far from the origin, and the
//! list of their clusters.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use bake::light::{linear_to_srgb, srgb_to_linear};
use bake::slicing::{LAYER, half_layer};
use egui_wgpu::wgpu;

use super::renderer::{Chunk, Frame};
use crate::engine::{Colours, Look, Painter};
use crate::gpu::Easel;
use crate::view::{self, Vec3, to_local, view_basis};

/** A chunk as the shader reads it (Chunk in compositor.wgsl). */
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuChunk {
    /** Top-left corner of what it draws, relative to the camera, face coordinates. */
    origin: [f32; 2],
    distance: f32,
    face: u32,
    /** Its level (depth_of in compositor.wgsl). */
    level: f32,
    /** RGBA to paint its voxels, shaded as they are (Colours); 0 for their own colours. */
    tint: u32,
    /** A half layer of its grid: the unit of its columns' and splats' depths. */
    half_layer: f32,
    /**
     * Its see-through splats whose cell of the next finer level is at most
     * this far past its plane, in half layers, that level draws: left out.
     */
    clip: f32,
    /** Drawn flat (Flat): its picture's q from its origin is drawn at scale · q + offset. */
    scale: [f32; 2],
    offset: [f32; 2],
    /** Its picture's plane past its own, z0 + gx x + gy y at (x, y) from its origin: z0, gx, gy; and Flat's `reach`. */
    plane: [f32; 4],
    /** Flat's `picture`, and room to keep the table's items 16 bytes apart. */
    picture: [u32; 2],
    unused: [u32; 2],
}

/** A column of a chunk's relief (columns in compositor.wgsl): its front's depth and its run, in half layers, as i16s, and its colour; 0, none. */
pub type GpuColumn = [u32; 2];

/** A splat (splat_vertex's and veil_vertex's instance in compositor.wgsl): its numbers as a cell's are baked (bake's bake_cell). */
pub type GpuSplat = [u32; 6];

/**
 * A cluster (Cluster in compositor.wgsl): a tile of a chunk's columns, or
 * splats of it. Where they are along the axis, relative to the
 * chunk's origin, and how far past the plane, nearest and farthest; a
 * tile's first column's centre, and the splats' widest's half width;
 * its first column or splat, its chunk's slot and how many splats, and
 * whether it showed in the last frame.
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
 * the slant of the rays to it (`enlarged`), at most the face's
 * (`MAX_SLANT`): neighbours overlap where their pillars would, and depth
 * decides between them.
 */
#[derive(Clone, Copy, Default)]
pub struct Flat {
    /** Its plane: past the chunk's plane by z0 + gx x + gy y at (x, y) from its origin, all in world units. */
    pub plane: [f64; 3],
    /** How far along the axis its edge columns' pillars reach from the plane, at most. */
    pub reach: f64,
    /** The box its columns' cells fill, from its origin. */
    pub min: [f64; 2],
    pub max: [f64; 2],
    /** Its picture's columns across and rows (flatten). */
    pub picture: [u32; 2],
}

/** The face's rays' tangents across and up, at most (|x| + √2 |y| ≤ z). */
const MAX_SLANT: [f64; 2] = [1.0, std::f64::consts::FRAC_1_SQRT_2];

/**
 * A chunk's relief drawn flat (Flat), from its columns as tiles() takes
 * them, as the GPU takes it: its picture, rows of colours (0, none), from
 * its columns' least even row and least column in a row, a row's columns
 * 2 apart in a, two colours to an item; and one cluster, a quad holding
 * the picture's box as far past the plane as the picture's plane is over
 * it, and as wide as it may be enlarged, which flat_vertex takes off again.
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
    let places: HashSet<[i32; 2]> = columns.iter().map(|c| c.0).collect();
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
    let cell = [LAYER * unit, 0.75 * unit];
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (_, centre, _) in &columns {
        for d in 0..2 {
            (min[d], max[d]) = (min[d].min(centre[d] - cell[d]), max[d].max(centre[d] + cell[d]));
        }
    }
    // Its picture: a column (a, b) is in row b, a's of its row 2 apart; the first row even, so that rows alternate as from (0, 0).
    let place = |[a, b]: [i32; 2]| [(a - (b & 1)).div_euclid(2), b];
    let (mut least, mut most) = ([i32::MAX; 2], [i32::MIN; 2]);
    for c in &columns {
        let p = place(c.0);
        (least, most) = ([0, 1].map(|d| least[d].min(p[d])), [0, 1].map(|d| most[d].max(p[d])));
    }
    least[1] &= !1;
    let [across, rows] = [0, 1].map(|d| (most[d] - least[d] + 1) as u32);
    let mut picture = vec![[0u32; 2]; (across * rows).div_ceil(2) as usize];
    for (c, (ab, ..)) in baked.iter().zip(&columns) {
        let [s, r] = place(*ab);
        let k = (r - least[1]) as usize * across as usize + (s - least[0]) as usize;
        picture[k / 2][k % 2] = c[2];
    }
    let flat = Flat { plane: [z0, g[0], g[1]], reach, min, max, picture: [across, rows] };
    let margin = MAX_SLANT.map(|s| s * reach);
    let first = [base[0] + (2 * least[0]) as f64 * LAYER * unit, base[1] + least[1] as f64 * unit];
    let mut quad = GpuCluster::new(0, 0, first);
    let depths = corners(min, max).map(|q| flat.depth(q));
    let centre = [0, 1].map(|d| (min[d] + max[d]) / 2.0);
    let half = [0, 1].map(|d| (max[d] - min[d]) / 2.0 + margin[d]);
    quad.hold(centre, half, [depths.into_iter().fold(f64::INFINITY, f64::min), depths.into_iter().fold(f64::NEG_INFINITY, f64::max)]);
    (picture, vec![quad], flat)
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

    /**
     * How it is enlarged, its origin (x, y) and its chunk's plane
     * `distance` from the camera: its picture's (x, y) is drawn at
     * scale · (x, y) + offset, each edge of its box out by reach times the
     * most the rays to the box slant at its nearest, at most the face's.
     */
    fn enlarged(&self, origin: [f64; 2], distance: f64) -> ([f64; 2], [f64; 2]) {
        let z = distance + corners(self.min, self.max).map(|q| self.depth(q)).into_iter().fold(f64::INFINITY, f64::min) - self.reach;
        let fit = |d: usize| {
            let far = (origin[d] + self.min[d]).abs().max((origin[d] + self.max[d]).abs());
            let slant = if z > 0.0 { (far / z).min(MAX_SLANT[d]) } else { MAX_SLANT[d] };
            let (centre, half) = ((self.min[d] + self.max[d]) / 2.0, (self.max[d] - self.min[d]) / 2.0);
            let scale = if half > 0.0 { 1.0 + slant * self.reach / half } else { 1.0 };
            (scale, centre * (1.0 - scale))
        };
        let [(sx, ox), (sy, oy)] = [fit(0), fit(1)];
        ([sx, sy], [ox, oy])
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
 * A chunk's splats as the GPU takes them, in clusters, and how many
 * clusters its whole ones take, which come first. The whole ones (where
 * its whole voxels are no relief) in clusters of the splats in each block
 * of size `block` on its plane, nearest first in each, so that a block's
 * first clusters are its front, the clusters nearest first, so that splats
 * are mostly drawn near to far; then the see-through ones, nearest first,
 * layer by layer, the order a ray of its face meets them in, and in a
 * layer by blocks. Their `first` counts from the chunk's first splat, and
 * their chunk's slot is not in them yet.
 * A cluster's box holds its splats' outlines along the axis (a
 * see-through one's tent, veil_fragment), from `near` to `far` past the
 * plane, `half_layer` per half layer of their depth, and how wide the
 * widest is: the GPU finds where they are drawn for its camera. In the
 * bake workers, so that a chunk's upload is a copy.
 */
pub fn cluster(numbers: &[f32], half_layer: f64, block: f64) -> (Vec<GpuSplat>, Vec<GpuCluster>, usize) {
    let numbers = numbers.as_chunks::<6>().0;
    let block_of = |s: usize| [1, 0].map(|d| (numbers[s][d] as f64 / block).floor() as i64);
    let depth = |s: usize| numbers[s][3] as i32;
    let whole = |s: &usize| numbers[*s][4].to_bits() >> 24 == 255;
    let (mut sorted, mut see_through): (Vec<usize>, Vec<usize>) = (0..numbers.len()).partition(whole);
    sorted.sort_by_key(|&s| (block_of(s), depth(s)));
    see_through.sort_by_key(|&s| (depth(s), block_of(s)));
    // A see-through splat's tent reaches twice as far as its outline.
    let of = |part: &[usize], first: usize, reach: f64| {
        let mut cluster = GpuCluster::new(first, part.len(), [0.0; 2]);
        for &s in part {
            let [x, y, w, depth, ..] = numbers[s].map(f64::from);
            let w = reach * w;
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
            clusters.push(of(&run[first..run.len() * (p + 1) / parts], start + first, 1.0));
        }
        start += run.len();
    }
    clusters.sort_by(|a, b| a.near.total_cmp(&b.near));
    let whole = clusters.len();
    for (k, part) in see_through.chunks(CLUSTER).enumerate() {
        clusters.push(of(part, start + k * CLUSTER, 2.0));
    }
    sorted.extend(see_through);
    (sorted.iter().map(|&s| numbers[s].map(f32::to_bits)).collect(), clusters, whole)
}

/** A chunk's columns, splats and clusters on the GPU, and its slot in the table. */
struct Resident {
    columns: Range<u32>,
    splats: Range<u32>,
    clusters: Range<u32>,
    /** Its first see-through cluster's. */
    see_through: u32,
    slot: u32,
}

/** A buffer of items `stride` bytes long, ranges of them allocated and freed. */
struct Arena {
    label: &'static str,
    stride: u64,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    /** Items the buffer holds, the end of those in use, and free ranges below it. */
    capacity: u32,
    top: u32,
    free: Vec<Range<u32>>,
    /** Whether the buffer grew this frame: its copy runs after the frame's writes, so free ranges below it wait. */
    grew: bool,
}

impl Arena {
    fn new(device: &wgpu::Device, label: &'static str, stride: u64, usage: wgpu::BufferUsages, capacity: u32) -> Arena {
        let usage = usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let capacity = capacity.min(Arena::most(device, stride, usage));
        let buffer = buffer(device, label, capacity as u64 * stride, usage);
        Arena { label, stride, usage, buffer, capacity, top: 0, free: Vec::new(), grew: false }
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

    /** Room for frames of `items` as the camera moves: three times that when it has less than twice, so that it seldom grows. */
    fn room(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, items: u32) {
        if 2 * items > self.capacity {
            self.grow(device, encoder, 3 * items);
        }
    }

    /** Grown to `capacity` items, as far as the device allows, what it holds copied over. */
    fn grow(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, capacity: u32) {
        let capacity = capacity.min(Arena::most(device, self.stride, self.usage));
        if capacity > self.capacity {
            let grown = buffer(device, self.label, capacity as u64 * self.stride, self.usage);
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &grown, 0, self.top as u64 * self.stride);
            (self.buffer, self.capacity, self.grew) = (grown, capacity, true);
        }
    }

    /** `count` items' room in the buffer, grown if need be; none when the device allows no more. */
    fn allocate(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, count: u32) -> Option<Range<u32>> {
        if count == 0 {
            return Some(0..0);
        }
        if let Some(k) = self.free.iter().position(|r| r.len() >= count as usize).filter(|_| !self.grew) {
            let r = self.free[k].clone();
            if r.len() == count as usize {
                self.free.remove(k);
            } else {
                self.free[k].start += count;
            }
            return Some(r.start..r.start + count);
        }
        if self.top + count > self.capacity {
            self.grow(device, encoder, (self.capacity * 2).max(self.top + count));
        }
        (self.top + count <= self.capacity).then(|| {
            self.top += count;
            self.top - count..self.top
        })
    }

    /** Frees a range, merging it with free neighbours. */
    fn release(&mut self, r: Range<u32>) {
        if r.is_empty() {
            return;
        }
        let at = self.free.partition_point(|f| f.start < r.start);
        self.free.insert(at, r);
        if at + 1 < self.free.len() && self.free[at].end == self.free[at + 1].start {
            self.free[at].end = self.free.remove(at + 1).end;
        }
        if at > 0 && self.free[at - 1].end == self.free[at].start {
            self.free[at - 1].end = self.free.remove(at).end;
        }
        if let Some(last) = self.free.pop_if(|f| f.end == self.top) {
            self.top = last.start;
        }
    }
}

/** What the compositor draws into, the size of the view in pixels. */
struct Targets {
    size: [u32; 2],
    picture: wgpu::Texture,
    depth: wgpu::Texture,
    /** The see-through splats over each other: their colour, and how much of the picture they let through (veil_fragment). */
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
    cull_veil: wgpu::BindGroupLayout,
    reduce_depth: wgpu::BindGroupLayout,
    reduce: wgpu::BindGroupLayout,
    show: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
    /** The tiles of whole voxels, or their splats; then the see-through splats over each other, and those over the picture. */
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
    cull_see_through: wgpu::ComputePipeline,
    reduce_depth_level: wgpu::ComputePipeline,
    reduce_level: wgpu::ComputePipeline,
}

pub struct Compositor {
    format: wgpu::TextureFormat,
    pipelines: Pipelines,
    globals: wgpu::Buffer,
    /** The chunk table, by slot. */
    table: wgpu::Buffer,
    slots: u32,
    free_slots: Vec<u32>,
    columns: Arena,
    splats: Arena,
    clusters: Arena,
    resident: HashMap<u32, Resident>,
    /**
     * The frame's chunks' clusters, [first, first + count) each and how
     * many of the frame's come before, after how many chunks and clusters
     * (compositor.wgsl's `candidate`).
     */
    candidates: wgpu::Buffer,
    /**
     * The frame's chunks' see-through clusters, as `candidates`, each a draw
     * in that order: finest level first, and within a level nearest first;
     * and how many draws.
     */
    veiled: wgpu::Buffer,
    veil_draws: wgpu::Buffer,
    veils: u32,
    /** The clusters in view. */
    in_view: u32,
    /** By phase, the draws of clusters the culling lists, and how many. */
    draws: [wgpu::Buffer; 2],
    drawn: [wgpu::Buffer; 2],
    targets: Option<Targets>,
    seams: bool,
    /** Whether the frame's whole voxels are reliefs, or splats (Frame's `relief`), and its reliefs flat (Frame's `flat`). */
    relief: bool,
    flat: bool,
    /** What draws, by phase what culls, and what culls the see-through. */
    draw_group: Option<wgpu::BindGroup>,
    cull_groups: Vec<wgpu::BindGroup>,
    veil_group: Option<wgpu::BindGroup>,
}

const SKY_TOP: [f32; 3] = [0x3f as f32 / 255.0, 0x73 as f32 / 255.0, 0xb8 as f32 / 255.0];
const SKY_HORIZON: [f32; 3] = [0xb9 as f32 / 255.0, 0xd3 as f32 / 255.0, 0xe8 as f32 / 255.0];
/** f32 in Globals (compositor.wgsl), and where its whole_splats is. */
pub const GLOBALS: usize = 220;
const WHOLE_SPLATS: usize = 6;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const FARTHEST: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const VEIL: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    buffer(device, label, size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST)
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
            &[(0, S::VERTEX_FRAGMENT, uniform), (1, S::VERTEX_FRAGMENT, read), (2, S::VERTEX_FRAGMENT, read), (3, S::FRAGMENT, read)],
        );
        let cull = layout(
            device,
            &[
                (0, S::COMPUTE, uniform),
                (1, S::COMPUTE, read),
                (4, S::COMPUTE, write),
                (5, S::COMPUTE, read),
                (6, S::COMPUTE, write),
                (7, S::COMPUTE, write),
                (8, S::COMPUTE, float),
            ],
        );
        let cull_veil = layout(
            device,
            &[
                (0, S::COMPUTE, uniform),
                (1, S::COMPUTE, read),
                (4, S::COMPUTE, write),
                (8, S::COMPUTE, float),
                (9, S::COMPUTE, read),
                (16, S::COMPUTE, write),
            ],
        );
        let reduce_depth = layout(device, &[(10, S::COMPUTE, texture(wgpu::TextureSampleType::Depth)), (12, S::COMPUTE, level_out)]);
        let reduce = layout(device, &[(11, S::COMPUTE, float), (12, S::COMPUTE, level_out)]);
        let show = layout(device, &[(13, S::FRAGMENT, float)]);
        let composite = layout(device, &[(14, S::FRAGMENT, float)]);
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
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4, 2 => Float32],
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
        // Nearest first, each under those before: its colour as much as they let through, and what it lets through.
        let under = wgpu::BlendState { color: blend(F::DstAlpha, F::One), alpha: blend(F::Zero, F::OneMinusSrcAlpha) };
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
            splats: render(&draw, "splat_vertex", "splat_fragment", &[Some(splat.clone())], tested.clone(), &[target(format, None)]),
            veil: render(&draw, "veil_vertex", "veil_fragment", &[Some(splat)], behind, &[target(VEIL, Some(under))]),
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
            cull_see_through: compute(&cull_veil, "cull_see_through", &[("culls", f64::from(culls))]),
            reduce_depth_level: compute(&reduce_depth, "reduce_depth", &[]),
            reduce_level: compute(&reduce, "reduce", &[]),
            draw,
            cull,
            cull_veil,
            reduce_depth,
            reduce,
            show,
            composite,
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
            table: storage(device, "chunks", 1024 * size_of::<GpuChunk>() as u64),
            slots: 0,
            free_slots: Vec::new(),
            columns: Arena::new(device, "columns", size_of::<GpuColumn>() as u64, wgpu::BufferUsages::STORAGE, 1 << 20),
            splats: Arena::new(device, "splats", size_of::<GpuSplat>() as u64, wgpu::BufferUsages::VERTEX, 1 << 20),
            clusters: Arena::new(device, "clusters", size_of::<GpuCluster>() as u64, wgpu::BufferUsages::STORAGE, 1 << 15),
            resident: HashMap::new(),
            candidates: storage(device, "candidates", 1024 * 16),
            veiled: storage(device, "see-through candidates", 1024 * 16),
            veil_draws: buffer(device, "see-through draws", 1 << 16, U::STORAGE | U::INDIRECT),
            veils: 0,
            in_view: 0,
            draws: [draws("draws first"), draws("draws next")],
            drawn: [drawn("drawn first"), drawn("drawn next")],
            targets: None,
            seams: false,
            relief: true,
            flat: false,
            draw_group: None,
            cull_groups: Vec::new(),
            veil_group: None,
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

    /** Uploads `chunk`'s columns, splats and clusters (see tiles and cluster), unless the buffers can hold no more. */
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, chunk: &Chunk) -> bool {
        let baked = &chunk.baked;
        let Some(columns) = self.columns.allocate(device, encoder, baked.columns.len() as u32) else { return false };
        let Some(splats) = self.splats.allocate(device, encoder, baked.splats.len() as u32) else {
            self.columns.release(columns);
            return false;
        };
        let Some(range) = self.clusters.allocate(device, encoder, baked.clusters.len() as u32) else {
            self.columns.release(columns);
            self.splats.release(splats);
            return false;
        };
        let slot = self.free_slots.pop().unwrap_or_else(|| {
            self.slots += 1;
            self.slots - 1
        });
        queue.write_buffer(&self.columns.buffer, columns.start as u64 * self.columns.stride, bytemuck::cast_slice(&baked.columns));
        queue.write_buffer(&self.splats.buffer, splats.start as u64 * self.splats.stride, bytemuck::cast_slice(&baked.splats));
        let clusters: Vec<GpuCluster> = baked
            .clusters
            .iter()
            .enumerate()
            .map(|(k, c)| {
                // Its whole voxels' first: tiles of columns, if the frame's are reliefs.
                let first = if k < baked.see_through && self.relief { columns.start } else { splats.start };
                GpuCluster { first: first + c.first, slot_count: slot << 8 | c.slot_count, ..*c }
            })
            .collect();
        queue.write_buffer(&self.clusters.buffer, range.start as u64 * self.clusters.stride, bytemuck::cast_slice(&clusters));
        let see_through = range.start + baked.see_through as u32;
        self.resident.insert(chunk.id, Resident { columns, splats, see_through, clusters: range, slot });
        true
    }

    /** Uploads what `frame` draws that is not on the GPU yet, drops what it no longer draws, and writes the chunk table, the clusters to cull and globals. */
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &Frame, view: &View) {
        (self.columns.grew, self.splats.grew, self.clusters.grew) = (false, false, false);
        let size = view.pixels.map(|v| v.max(1));
        (self.seams, self.relief, self.flat) = (view.seams, frame.relief, frame.flat);
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(Targets::new(device, &self.pipelines, self.format, size));
        }
        let mut table = Vec::new();
        // What the frame no longer draws makes room first.
        let wanted: HashSet<u32> = frame.chunks.iter().map(|p| p.chunk.id).collect();
        let gone: Vec<u32> = self.resident.keys().filter(|id| !wanted.contains(id)).copied().collect();
        for id in gone {
            let r = self.resident.remove(&id).expect("resident");
            self.free_slots.push(r.slot);
            self.columns.release(r.columns);
            self.splats.release(r.splats);
            self.clusters.release(r.clusters);
        }
        // Growing copies all a buffer holds, so grow seldom.
        let chunks = || frame.chunks.iter().map(|p| &p.chunk.baked);
        self.columns.room(device, encoder, chunks().map(|b| b.columns.len() as u32).sum());
        self.splats.room(device, encoder, chunks().map(|b| b.splats.len() as u32).sum());
        self.clusters.room(device, encoder, chunks().map(|b| b.clusters.len() as u32).sum());
        let mut candidates: Vec<[u32; 4]> = vec![[0; 4]];
        // Each with its level and how far its site is from the eye.
        let mut veiled: Vec<([u32; 4], (i32, f64))> = Vec::new();
        let mut full = false;
        for p in &frame.chunks {
            // A hue per layer of a level's cells (Colours), neighbours in a stack far apart.
            let cell = &p.cell;
            let hue = hash(&[p.face as u64, cell.level as u64]) + 0.618 * (cell.site[2] / cell.spacing()) as f64;
            let chunk = &p.chunk;
            // Chunks come finest and nearest first: once the buffers are full, what is farther waits.
            if !self.resident.contains_key(&chunk.id) && (full || !self.upload(device, queue, encoder, chunk)) {
                full = true;
                continue;
            }
            let resident = &self.resident[&chunk.id];
            let slot = resident.slot as usize;
            if table.len() <= slot {
                table.resize(slot + 1, GpuChunk::default());
            }
            let flat = &chunk.baked.flat;
            let (scale, offset) = flat.enlarged([p.x, p.y], p.distance);
            table[slot] = GpuChunk {
                origin: [p.x as f32, p.y as f32],
                distance: p.distance as f32,
                face: p.face as u32,
                level: cell.level as f32,
                tint: match view.colours {
                    Colours::Voxels => 0,
                    Colours::Layers => tint(hue),
                    Colours::Chunks => tint(hash(&[hue.to_bits(), cell.site[0] as u64, cell.site[1] as u64])),
                },
                half_layer: half_layer(cell.atoms) as f32,
                // In its grid's units, of which reach's layers are 2^atoms.
                clip: p.finer.map_or(f32::NEG_INFINITY, |r| (2 * ((r >> cell.atoms) - cell.site[2] as i64)) as f32),
                scale: scale.map(|v| v as f32),
                offset: offset.map(|v| v as f32),
                plane: [flat.plane[0], flat.plane[1], flat.plane[2], flat.reach].map(|v| v as f32),
                picture: flat.picture,
                unused: [0; 2],
            };
            let Resident { clusters, see_through, .. } = resident;
            let before = candidates.last().map_or(0, |c| c[1] + c[2]);
            candidates.push([clusters.start, see_through - clusters.start, before, 0]);
            if *see_through < clusters.end {
                veiled.push(([*see_through, clusters.end - see_through, 0, 0], (cell.level, p.from_eye)));
            }
        }
        // A ray of a face meets finer levels first (lod.rs), and within a level the cells of the lattice's Voronoi
        // cells nearer the eye: at a face between two, the nearer one's site is nearer.
        veiled.sort_by(|a, b| a.1.0.cmp(&b.1.0).then(a.1.1.total_cmp(&b.1.1)));
        let mut veils = 0;
        let mut veiled: Vec<[u32; 4]> = [[0; 4]]
            .into_iter()
            .chain(veiled.into_iter().map(|([first, count, ..], _)| {
                veils += count;
                [first, count, veils - count, 0]
            }))
            .collect();
        at_least(device, &mut self.table, self.slots as u64 * size_of::<GpuChunk>() as u64);
        if !table.is_empty() {
            queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        }
        self.in_view = candidates.iter().map(|c| c[1]).sum();
        self.veils = veils;
        candidates[0] = [candidates.len() as u32 - 1, self.in_view, 0, 0];
        veiled[0] = [veiled.len() as u32 - 1, veils, 0, 0];
        at_least(device, &mut self.candidates, candidates.len() as u64 * 16);
        queue.write_buffer(&self.candidates, 0, bytemuck::cast_slice(&candidates));
        at_least(device, &mut self.veiled, veiled.len() as u64 * 16);
        queue.write_buffer(&self.veiled, 0, bytemuck::cast_slice(&veiled));
        at_least(device, &mut self.veil_draws, veils.max(1) as u64 * 16);
        for draws in &mut self.draws {
            at_least(device, draws, self.in_view.max(1) as u64 * 16);
        }
        let sky_fog = 1.0 - (-frame.farthest / view.fog_distance).exp();
        let mut globals = view.globals(size, sky_fog);
        globals[WHOLE_SPLATS] = if frame.relief { 0.0 } else { 1.0 };
        queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));
        self.bind_groups(device);
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
                    (5, self.candidates.as_entire_binding()),
                    (6, self.draws[k].as_entire_binding()),
                    (7, self.drawn[k].as_entire_binding()),
                    (8, R::TextureView(&farthest)),
                ];
                bind(device, &self.pipelines.cull, &entries)
            })
            .collect();
        let entries = [
            (0, self.globals.as_entire_binding()),
            (1, self.table.as_entire_binding()),
            (4, self.clusters.buffer.as_entire_binding()),
            (8, R::TextureView(&farthest)),
            (9, self.veiled.as_entire_binding()),
            (16, self.veil_draws.as_entire_binding()),
        ];
        self.veil_group = Some(bind(device, &self.pipelines.cull_veil, &entries));
    }

    /**
     * Draws the frame prepared last into the picture: the tiles (or
     * clusters of whole splats) that showed last frame, then those that show now that did not, culled
     * against the farthest depths of the first; then the see-through ones
     * in front of them, nearest first, and those over the picture.
     */
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder) {
        let t = self.targets.as_ref().expect("prepared");
        let p = &self.pipelines;
        for drawn in &self.drawn {
            encoder.clear_buffer(drawn, 0, None);
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
            if self.in_view > 0 {
                pass.set_pipeline(if phase == 0 { &p.cull_drawn } else { &p.cull_rest });
                pass.set_bind_group(0, &self.cull_groups[phase], &[]);
                let [across, rows] = groups(self.in_view.div_ceil(CULL_THREADS));
                pass.dispatch_workgroups(across, rows, 1);
            }
            if phase == 1 && self.veils > 0 {
                pass.set_pipeline(&p.cull_see_through);
                pass.set_bind_group(0, self.veil_group.as_ref(), &[]);
                let [across, rows] = groups(self.veils.div_ceil(CULL_THREADS));
                pass.dispatch_workgroups(across, rows, 1);
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
            pass.multi_draw_indirect_count(&self.draws[phase], 0, &self.drawn[phase], 0, self.in_view);
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
            // Nothing yet, all let through.
            let mut veil = pass(encoder, &[(&t.veil, Some(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }))], (&t.depth, None));
            veil.set_bind_group(0, self.draw_group.as_ref(), &[]);
            veil.set_pipeline(&p.veil);
            veil.set_vertex_buffer(0, self.splats.buffer.slice(..));
            veil.multi_draw_indirect(&self.veil_draws, 0, self.veils);
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
            sky: look.sky.unwrap_or(1.0),
        }
    }

    pub fn globals(&self, pixels: [u32; 2], sky_fog: f64) -> [f32; GLOBALS] {
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
            let at = 28 + f * 16;
            columns(&mut g[at..at + 12], axes.map(|axis| to_local(face, axis)));
            g[at + 12..at + 15].copy_from_slice(&face[2].map(|v| (v * std::f64::consts::SQRT_2).round() as f32));
        }
        g
    }
}

impl Painter for Compositor {
    fn picture(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("a frame").picture
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
            "flat pictures"
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
    fn groups_hold_every_chunk_in_rows_a_dispatch_allows() {
        let most = wgpu::Limits::default().max_compute_workgroups_per_dimension;
        for n in [0, 1, 1000, most, most + 1, 3 * most + 7] {
            let [across, rows] = groups(n);
            // Within the limit, all of them, and at most a workgroup a row to spare.
            assert!(across <= most && across * rows >= n && across * rows < n + rows, "{n}: {across} × {rows}");
        }
    }
}
