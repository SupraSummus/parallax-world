//! Renders the world as stacks of cached slices split into chunks
//! (renderer.ts): each render picks the chunks in view, face by face and
//! near to far, has the missing ones baked and returns where each goes,
//! relative to the camera. A cell is baked once the world has all the
//! tiles it reads; until then, the tiles are what it wants. Cells nearer
//! layers cover want neither, as the occlusion thread finds from what
//! earlier renders saw (occlusion.rs); drawing what is in view, and culling
//! what of it is hidden, is the compositor's job.
//! See docs/architecture.md, sections 3 to 5.

use std::collections::HashMap;
use std::sync::Arc;

use bake::slicing::{GRID_LEVELS, LAYER, cell_margin, chunk_cell, chunk_cells, leak_free_detail, select_slices, site_layers, slice_centre};
use bake::{Lattice, Slice, chunk_reads};

use crate::bakery::{Baked, Bakery, Request};
use crate::occlusion::{self, CellKey, Occlusion, Seeing, Seen};
use crate::view::{Face, ScreenBox, ScreenBoxes, Vec3, View, in_face, to_local};
use crate::world::{Level, TILE, TileId};

/** Slices start four layers ahead: a nearer one would blow single voxels up over the screen. */
const NEAR_DEPTH: f64 = 4.0 * LAYER;

/** Slices unused for this many renders are dropped once there are more than MAX_SLICES. */
const MAX_SLICES: usize = 2048;
const UNUSED_RENDERS: u64 = 300;

/** A baked piece of a slice: cell (i, j). `id` names it to the coverage and the compositor. */
pub struct Chunk {
    pub id: u32,
    pub i: i32,
    pub j: i32,
    pub baked: Baked,
}

/** A chunk in a frame: its top-left corner on its layer's plane, relative to the camera, in the face's coordinates. */
pub struct Placement {
    pub chunk: Arc<Chunk>,
    pub x: f64,
    pub y: f64,
}

/** A slice's centre plane, `distance` ahead of the camera along its face's axis, and what it draws there. */
pub struct Layer {
    pub face: usize,
    pub slice: Slice,
    pub distance: f64,
    pub placements: Vec<Placement>,
}

/** What to draw: layers face by face, near to far. */
pub struct Frame {
    pub layers: Vec<Layer>,
    /** The farthest layer's distance, or the render distance if farther: the sky's fog. */
    pub farthest: f64,
}

#[derive(Default)]
pub struct Stats {
    pub chunks: usize,
    /** Chunks in view to bake, nearest first. */
    pub wanted: Vec<Request>,
    /** Cells covered by nearer layers, so not baked, and chunks in view hidden, as the latest occlusion pass found. */
    pub covered: usize,
    /** Whether an occlusion pass for this view is yet to come, or cells in view wait for one to say whether they are covered. */
    pub occluding: bool,
    /** The grid levels the frame's slices read, whether they have tiles yet or not. */
    pub levels: [bool; GRID_LEVELS],
    /** Tiles that cells in view read and the world has not made, by grid level, most wanted first. */
    pub tiles: Vec<(usize, TileId)>,
}

struct CachedSlice {
    id: Slice,
    /** Its voxels may be in its face's layers [depth, depth + size). */
    depth: i64,
    size: i64,
    /** By cell: its chunk, once baked. */
    cells: HashMap<(i32, i32), Arc<Chunk>>,
    /** By cell: the tiles it reads (see cell_tiles). */
    tiles: HashMap<(i32, i32), Option<[i32; 4]>>,
    /** By cell, once it has its tiles: the least and greatest b of the voxels its chunks may draw, or None if none (see cell_margin). */
    extents: HashMap<(i32, i32), Option<(f64, f64)>>,
    used: u64,
}

type SliceKey = (usize, i32, i32, i32, u64);

fn slice_key(face: usize, s: &Slice) -> SliceKey {
    (face, s.level, s.atoms, s.layer, s.cutoff.to_bits())
}

pub struct Renderer {
    faces: Vec<Face>,
    /** Per grid level, its tiles made so far, and the y of its sites, [low, high). */
    levels: Vec<Arc<Level>>,
    heights: [(i64, i64); GRID_LEVELS],
    slices: HashMap<SliceKey, CachedSlice>,
    occlusion: Occlusion,
    bakery: Bakery,
    next_id: u32,
    renders: u64,
    /** Degrees: how far hidden geometry may show between slices. */
    pub max_leak: f64,
    pub render_distance: f64,
    pub skip_covered: bool,
    /** Whether runs are drawn as pillars: chunks baked otherwise are baked again, standing in meanwhile. */
    pub pillars: bool,
}

/** One face as seen in one render. */
struct FaceView<'a> {
    face: usize,
    lattice: &'a Lattice,
    tangents: [f64; 4],
    camera: Vec3,
    boxes: ScreenBoxes,
}

/** |v| for the v in [lo, hi] nearest to 0. */
fn closest_to_zero(lo: f64, hi: f64) -> f64 {
    if lo > 0.0 {
        lo
    } else if hi < 0.0 {
        -hi
    } else {
        0.0
    }
}

impl Renderer {
    /** A renderer of a world whose sites of each grid level have y in [low, high) of `heights`. */
    pub fn new(faces: Vec<Face>, bakery: Bakery, render_distance: f64, heights: [(i64, i64); GRID_LEVELS]) -> Renderer {
        Renderer {
            faces,
            levels: (0..GRID_LEVELS).map(|_| Arc::new(Level::new(HashMap::new()))).collect(),
            heights,
            slices: HashMap::new(),
            occlusion: Occlusion::new(),
            bakery,
            next_id: 0,
            renders: 0,
            max_leak: 6.0,
            render_distance,
            skip_covered: true,
            pillars: true,
        }
    }

    /** Draws the tiles of grid level `level` from now on: cells that lacked them can be baked. */
    pub fn set_tiles(&mut self, level: usize, tiles: Arc<Level>) {
        self.bakery.set_tiles(level, tiles.grids.clone());
        self.levels[level] = tiles;
    }

    /** Waits until the bakery delivers a chunk. */
    pub fn wait(&mut self) {
        self.bakery.wait();
    }

    /** Waits until the occlusion thread has passed over the last render. */
    pub fn wait_for_occlusion(&self) {
        self.occlusion.wait();
    }

    /** How long the latest occlusion pass took, in seconds. */
    pub fn occlusion_seconds(&self) -> f64 {
        self.occlusion.answer().0.seconds
    }

    pub fn render(&mut self, camera: Vec3, view: &View) -> (Frame, Stats) {
        self.receive();
        self.renders += 1;
        let detail = leak_free_detail(self.max_leak.to_radians());
        let (answer, _) = self.occlusion.answer();
        let mut frame = Frame { layers: Vec::new(), farthest: self.render_distance };
        let mut stats = Stats { covered: if self.skip_covered { answer.hidden } else { 0 }, ..Stats::default() };
        let mut missing: Vec<(Request, f64)> = Vec::new();
        let mut tiles: Vec<(usize, TileId, f64)> = Vec::new();
        let mut seeing = Seeing { width: view.width, height: view.height, focal: view.focal, faces: Vec::new() };
        for f in 0..self.faces.len() {
            let face = &self.faces[f];
            let Some(tangents) = view.face_tangents(face) else { continue };
            let camera = to_local(&face.axes, camera);
            let mut seen = occlusion::Face { face: f, basis: view.face_basis(face), layers: Vec::new() };
            let of = FaceView { face: f, lattice: &face.lattice, tangents, camera, boxes: view.screen_boxes(face) };
            for id in select_slices(camera[2] / LAYER, NEAR_DEPTH / LAYER, self.render_distance / LAYER, detail) {
                let key = slice_key(f, &id);
                let slice = self.slices.entry(key).or_insert_with(|| {
                    let (first, last) = site_layers(id.level, id.atoms);
                    let depth = id.layer as i64 * (1i64 << id.level) + first as i64;
                    let size = (last - first + 1) as i64;
                    CachedSlice { id, depth, size, cells: HashMap::new(), tiles: HashMap::new(), extents: HashMap::new(), used: 0 }
                });
                slice.used = self.renders;
                let level = id.atoms as usize;
                stats.levels[level] = true;
                let source = Source { level, tiles: &self.levels[level], heights: self.heights[level] };
                let (layer, cells, asking) = draw_slice(&of, slice, &source, view, self.pillars);
                // What cells ask for, once the latest pass found them uncovered.
                for (key, asks) in asking {
                    match if self.skip_covered { answer.covered.get(&key).copied() } else { Some(false) } {
                        Some(false) => {
                            missing.extend(asks.chunk);
                            tiles.extend(asks.tiles);
                        }
                        Some(true) => {}
                        None => stats.occluding = true,
                    }
                }
                if self.skip_covered {
                    seen.layers.push(occlusion::Layer { distance: layer.distance, cells });
                }
                frame.farthest = frame.farthest.max(layer.distance);
                stats.chunks += layer.placements.len();
                frame.layers.push(layer);
            }
            seeing.faces.push(seen);
        }
        if self.skip_covered {
            self.occlusion.ask(seeing);
            stats.occluding |= self.occlusion.answer().1;
        }
        // Near to far.
        missing.sort_by(|a, b| a.1.total_cmp(&b.1));
        stats.wanted = missing.iter().map(|m| m.0).collect();
        tiles.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut listed = std::collections::HashSet::new();
        stats.tiles = tiles.into_iter().filter(|t| listed.insert((t.0, t.1))).map(|t| (t.0, t.1)).collect();
        self.bakery.want(&stats.wanted);
        self.evict();
        (frame, stats)
    }

    /**
     * Caches what the bakery delivered, unless its slice was dropped
     * meanwhile, or its cell has a chunk already that it does not replace:
     * one with pillars as they are to be drawn now, or the delivered one
     * has not.
     */
    fn receive(&mut self) {
        for (r, baked) in self.bakery.take() {
            let Some(slice) = self.slices.get_mut(&slice_key(r.face, &r.slice)) else { continue };
            let pillars = self.pillars;
            if slice.cells.get(&(r.i, r.j)).is_some_and(|c| c.baked.pillars == pillars || r.pillars != pillars) {
                continue;
            }
            self.next_id += 1;
            slice.cells.insert((r.i, r.j), Arc::new(Chunk { id: self.next_id, i: r.i, j: r.j, baked }));
        }
    }

    fn evict(&mut self) {
        if self.slices.len() > MAX_SLICES {
            let renders = self.renders;
            self.slices.retain(|_, s| s.used + UNUSED_RENDERS > renders);
        }
    }
}

/** Where a slice's cells read from: its grid level, the tiles made of it, and the y of its sites, [low, high). */
struct Source<'a> {
    level: usize,
    tiles: &'a Level,
    heights: (i64, i64),
}

/** What a cell wants: its chunk baked, and its distance; tiles made, by grid level, and its distance. */
#[derive(Default)]
struct Asks {
    chunk: Option<(Request, f64)>,
    tiles: Vec<(usize, TileId, f64)>,
}

/**
 * The tiles chunk (i, j) of a slice reads (chunk_reads), [i0, k0, i1, k1]
 * (those its box of sites meets), or None if it reads none, the box being
 * all above or below the world. Found on first use.
 */
fn cell_tiles(of: &FaceView, slice: &mut CachedSlice, heights: (i64, i64), i: i32, j: i32) -> Option<[i32; 4]> {
    *slice.tiles.entry((i, j)).or_insert_with(|| {
        let (_, min, max) = chunk_reads(&slice.id, i, j);
        let Lattice { r, u, f } = *of.lattice;
        // x, y, z at (a, b, c) as bake's Reader has it, over the box's corners.
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for corner in 0..8 {
            let [a, b, c] = std::array::from_fn(|d| if corner >> d & 1 == 0 { min[d] } else { max[d] } as f64);
            for d in 0..3 {
                let v = (a * r[d] as f64 + c * f[d] as f64) / 2.0 + b * u[d] as f64;
                lo[d] = lo[d].min(v);
                hi[d] = hi[d].max(v);
            }
        }
        if hi[1] < heights.0 as f64 || lo[1] >= heights.1 as f64 {
            return None;
        }
        let tile = |v: f64| (v / TILE as f64).floor() as i32;
        Some([tile(lo[0]), tile(lo[2]), tile(hi[0]), tile(hi[2])])
    })
}

/** The least and greatest b of the voxels chunk (i, j) of a slice may draw (cellExtent in renderer.ts), once it has the tiles it reads. */
fn cell_extent(of: &FaceView, slice: &mut CachedSlice, level: &Level, [i0, k0, i1, k1]: [i32; 4], i: i32, j: i32) -> Option<(f64, f64)> {
    *slice.extents.entry((i, j)).or_insert_with(|| {
        let cell = chunk_cell(slice.id.level, i, j);
        // The cell's sites, in lattice units.
        let (left, right) = (cell.left as i64, (cell.left + cell.columns) as i64);
        let (columns, rows) = cell_margin(slice.id.level, slice.id.atoms);
        // In the units of the grid the slice reads, N times larger.
        let n = 1i64 << slice.id.atoms;
        let ceil = |v: i64| -(-v).div_euclid(n);
        let (c0, c1) = (slice.depth.div_euclid(n), ceil(slice.depth + slice.size));
        let (a0, a1) = ((left - columns as i64).div_euclid(n), ceil(right + columns as i64));
        let (lo, hi) = (k0..=k1)
            .flat_map(|k| (i0..=i1).map(move |i| (i, k)))
            .filter_map(|t| level.tiles.get(&t)?.index.range(of.lattice.f, c0, c1, a0, a1))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))?;
        let (bottom, top) = (cell.bottom as f64, (cell.bottom + cell.rows) as f64);
        let lo = ((lo * n) as f64).max(bottom - rows as f64);
        let hi = ((hi * n) as f64).min(top + rows as f64);
        (lo <= hi).then_some((lo, hi))
    })
}

/**
 * The layer a slice draws (drawSlice in renderer.ts): every chunk in view;
 * and its cells in view as the occlusion thread wants them, with what each
 * asks for, if anything: its chunk baked, or the tiles it lacks.
 */
fn draw_slice(
    of: &FaceView,
    slice: &mut CachedSlice,
    source: &Source,
    view: &View,
    pillars: bool,
) -> (Layer, Vec<Seen>, Vec<(CellKey, Asks)>) {
    let id = slice.id;
    let camera = of.camera;
    // Slices start at least NEAR_DEPTH ahead of the camera, so this is > 0.
    let distance = slice_centre(&id) - camera[2];
    let mut layer = Layer { face: of.face, slice: id, distance, placements: Vec::new() };
    let (mut cells, mut asking) = (Vec::new(), Vec::new());
    let [min_x, max_x, min_y, max_y] = of.tangents;
    let (left, right) = (camera[0] + min_x * distance, camera[0] + max_x * distance);
    let (bottom, top) = (camera[1] + min_y * distance, camera[1] + max_y * distance);
    let reach = cell_margin(id.level, id.atoms).1 as f64;
    let shown = |b: Option<ScreenBox>| b.filter(|b| view.on_screen(b));
    for (i, j) in chunk_cells(id.level, left, right, bottom, top) {
        let cell = chunk_cell(id.level, i, j);
        let (x, y) = (cell.left as f64 * LAYER, cell.bottom as f64);
        let Some(tiles) = cell_tiles(of, slice, source.heights, i, j) else { continue };
        let [i0, k0, i1, k1] = tiles;
        let complete = source.tiles.grids.has_all(tiles);
        // Once it has its tiles: nothing to draw if not baked at all; otherwise, what it draws lies within reach of its voxels.
        let (y0, y1) = if complete {
            let Some(extent) = cell_extent(of, slice, source.tiles, tiles, i, j) else { continue };
            (y.max(extent.0 - reach), (y + cell.height).min(extent.1 + reach))
        } else {
            (y, y + cell.height)
        };
        let (x0, x1) = (x - camera[0], x + cell.width - camera[0]);
        let (y0, y1) = (y0 - camera[1], y1 - camera[1]);
        if y0 >= y1 || !in_face(closest_to_zero(x0, x1), closest_to_zero(y0, y1), distance) {
            continue;
        }
        let Some(cell_box) = shown(of.boxes.of(x0, x1, y0, y1, distance)) else { continue };
        let mut asks = Asks::default();
        if !complete {
            for k in k0..=k1 {
                for i in i0..=i1 {
                    if !source.tiles.grids.has(i, k) {
                        asks.tiles.push((source.level, (i, k), distance));
                    }
                }
            }
        }
        // Baked once its tiles are in; kept once baked, though its tiles go;
        // baked again if its pillars are to be drawn otherwise, standing in meanwhile.
        let chunk = slice.cells.get(&(i, j)).cloned();
        if chunk.as_ref().is_none_or(|c| c.baked.pillars != pillars) && complete {
            let block = crate::compositor::block(distance, view.focal);
            asks.chunk = Some((Request { face: of.face, slice: id, i, j, tiles, block, pillars }, distance));
        }
        // The chunk's content, not just its cell, must reach the screen.
        let placement = chunk.filter(|c| !c.baked.splats.is_empty()).and_then(|chunk| {
            let (x, y) = (chunk.baked.origin[0] - camera[0], chunk.baked.origin[1] - camera[1]);
            let [width, height] = chunk.baked.extent;
            shown(of.boxes.of(x, x + width, y - height, y, distance)).map(|_| Placement { chunk, x, y })
        });
        let asks = (asks.chunk.is_some() || !asks.tiles.is_empty()).then_some(asks);
        if placement.is_none() && asks.is_none() {
            continue;
        }
        let key = (of.face, id.level, id.layer, id.cutoff.to_bits(), i, j);
        cells.push(Seen {
            key,
            screen: cell_box,
            rect: [x0, y - camera[1], cell.width, cell.height],
            shown: placement.as_ref().map(|p| (p.chunk.clone(), p.x, p.y)),
            asks: asks.is_some(),
        });
        asking.extend(asks.map(|asks| (key, asks)));
        layer.placements.extend(placement);
    }
    (layer, cells, asking)
}
