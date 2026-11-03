//! Renders the world as stacks of flat, cached slices split into chunks
//! (renderer.ts): each render picks the chunks in view, face by face and
//! near to far, skips those nearer layers cover, has the missing ones baked
//! (an older bake of a cell standing in meanwhile) and returns where each
//! goes, relative to the camera. Drawing them is the compositor's job.
//! See docs/architecture.md, sections 3 to 5.

use std::collections::HashMap;
use std::sync::Arc;

use bake::{Coverage, Slice};

use crate::bakery::{Baked, Bakery, Mask, Request};
use crate::layer_index::LayerIndex;
use crate::slicing::{
    GRID_LEVELS, LAYER, MAX_LEVEL, atom_level, cell_margin, chunk_cell, chunk_cells, eye_of, leak_free_detail, select_slices, site_layers,
    slice_centre,
};
use crate::view::{Face, ScreenBox, ScreenBoxes, Vec3, View, in_face, to_local};

/** Slices start four layers ahead: the nearest voxels then reach three layers ahead, two past level 0's eye. */
const NEAR_DEPTH: f64 = 4.0 * LAYER;

/** Slices unused for this many renders are dropped once there are more than MAX_SLICES. */
const MAX_SLICES: usize = 2048;
const UNUSED_RENDERS: u64 = 300;

/** Chunks kept per cell: the latest and the one before, so going back and forth over an eye's edge rebakes nothing. */
const CHUNKS_PER_CELL: usize = 2;

/** A baked piece of a slice: cell (i, j), from `eye`, grown if `fill`. `id` names it to the coverage and the compositor. */
pub struct Chunk {
    pub id: u32,
    i: i32,
    j: i32,
    eye: [i32; 3],
    fill: bool,
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
    /** Chunks in view to bake, most wanted first; an older bake of the cell, if any, stands in meanwhile. */
    pub wanted: Vec<Request>,
    /** Cells and chunks covered by nearer layers, so neither drawn nor baked. */
    pub covered: usize,
}

#[derive(Default)]
struct Cell {
    chunks: Vec<Arc<Chunk>>,
}

impl Cell {
    fn find(&self, eye: [i32; 3], fill: bool) -> Option<&Arc<Chunk>> {
        self.chunks.iter().find(|c| c.eye == eye && c.fill == fill)
    }

    fn latest(&self) -> Option<&Arc<Chunk>> {
        self.chunks.last()
    }
}

struct CachedSlice {
    id: Slice,
    /** Its voxels may be in its face's layers [depth, depth + size). */
    depth: i64,
    size: i64,
    cells: HashMap<(i32, i32), Cell>,
    /** By cell: the least and greatest b of the voxels its chunks may draw, or None if none (see cell_margin). */
    extents: HashMap<(i32, i32), Option<(f64, f64)>>,
    used: u64,
}

type SliceKey = (usize, i32, i32, u64);

fn slice_key(face: usize, s: &Slice) -> SliceKey {
    (face, s.level, s.layer, s.cutoff.to_bits())
}

pub struct Renderer {
    faces: Vec<Face>,
    /** Per grid level, where its voxels are; None while it has no grid. */
    indices: Vec<Option<LayerIndex>>,
    slices: HashMap<SliceKey, CachedSlice>,
    coverage: Coverage,
    bakery: Bakery,
    next_id: u32,
    renders: u64,
    /** Degrees: how far hidden geometry may show between slices. */
    pub max_leak: f64,
    pub render_distance: f64,
    pub fill: bool,
    pub skip_covered: bool,
}

/** One face as seen in one render. */
struct FaceView<'a> {
    face: usize,
    lattice: &'a [[i32; 3]; 3],
    tangents: [f64; 4],
    camera: Vec3,
    eyes: Vec<[i32; 3]>,
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
    pub fn new(faces: Vec<Face>, indices: Vec<Option<LayerIndex>>, bakery: Bakery, render_distance: f64) -> Renderer {
        assert_eq!(indices.len(), GRID_LEVELS);
        Renderer {
            faces,
            indices,
            slices: HashMap::new(),
            coverage: Coverage::default(),
            bakery,
            next_id: 0,
            renders: 0,
            max_leak: 1.5,
            render_distance,
            fill: true,
            skip_covered: true,
        }
    }

    /** Waits until the bakery delivers a chunk. */
    pub fn wait(&mut self) {
        self.bakery.wait();
    }

    pub fn render(&mut self, camera: Vec3, view: &View) -> (Frame, Stats) {
        self.receive();
        self.renders += 1;
        let detail = leak_free_detail(self.max_leak.to_radians());
        if self.skip_covered {
            self.coverage.set_view(view.width, view.height, view.focal);
        }
        let mut frame = Frame { layers: Vec::new(), farthest: self.render_distance };
        let mut stats = Stats::default();
        let mut missing: Vec<(Request, f64, bool)> = Vec::new();
        for f in 0..self.faces.len() {
            let face = &self.faces[f];
            let Some(tangents) = view.face_tangents(face) else { continue };
            let camera = to_local(&face.axes, camera);
            if self.skip_covered {
                let [r, u, fw] = view.face_basis(face);
                self.coverage.set_face(f, r, u, fw);
            }
            let of = FaceView {
                face: f,
                lattice: &face.lattice,
                tangents,
                camera,
                eyes: (0..=MAX_LEVEL).map(|level| eye_of(level, camera)).collect(),
                boxes: view.screen_boxes(face),
            };
            for id in select_slices(camera[2] / LAYER, NEAR_DEPTH / LAYER, self.render_distance / LAYER, detail) {
                let key = slice_key(f, &id);
                let slice = self.slices.entry(key).or_insert_with(|| {
                    let (first, last) = site_layers(id.level);
                    let depth = id.layer as i64 * (1i64 << id.level) + first as i64;
                    CachedSlice { id, depth, size: (last - first + 1) as i64, cells: HashMap::new(), extents: HashMap::new(), used: 0 }
                });
                slice.used = self.renders;
                let index = self.indices[atom_level(id.level) as usize].as_ref();
                let coverage = self.skip_covered.then_some(&mut self.coverage);
                let layer = draw_slice(&of, slice, index, coverage, view, self.fill, &mut missing, &mut stats);
                frame.farthest = frame.farthest.max(layer.distance);
                if self.skip_covered {
                    for p in &layer.placements {
                        let cell = chunk_cell(id.level, p.chunk.i, p.chunk.j);
                        self.coverage.cover(
                            p.chunk.id,
                            p.x,
                            p.y,
                            cell.x - camera[0],
                            cell.y - camera[1],
                            cell.width,
                            cell.height,
                            layer.distance,
                        );
                    }
                    self.coverage.end_layer(layer.distance);
                }
                stats.chunks += layer.placements.len();
                frame.layers.push(layer);
            }
        }
        // Cells that show nothing first, then the rest, near to far within each.
        missing.sort_by(|a, b| a.2.cmp(&b.2).then(a.1.total_cmp(&b.1)));
        stats.wanted = missing.iter().map(|m| m.0).collect();
        self.bakery.want(&stats.wanted);
        self.evict();
        (frame, stats)
    }

    /** Caches what the bakery delivered, unless its slice was dropped meanwhile. */
    fn receive(&mut self) {
        for (r, baked) in self.bakery.take() {
            let Some(slice) = self.slices.get_mut(&slice_key(r.face, &r.slice)) else { continue };
            let cell = slice.cells.entry((r.i, r.j)).or_default();
            if cell.find(r.eye, r.fill).is_some() {
                continue;
            }
            self.next_id += 1;
            cell.chunks.push(Arc::new(Chunk { id: self.next_id, i: r.i, j: r.j, eye: r.eye, fill: r.fill, baked }));
            if cell.chunks.len() > CHUNKS_PER_CELL {
                cell.chunks.remove(0);
            }
        }
    }

    fn evict(&mut self) {
        if self.slices.len() > MAX_SLICES {
            let renders = self.renders;
            self.slices.retain(|_, s| s.used + UNUSED_RENDERS > renders);
        }
    }
}

/** Hands a chunk's mask to the coverage unless it has it. */
fn known(coverage: &mut Coverage, chunk: &Chunk) -> u32 {
    if !coverage.use_chunk(chunk.id) {
        let Mask { width, height, scale, ref blocks } = chunk.baked.mask;
        coverage.add(chunk.id, width, height, scale, blocks);
    }
    chunk.id
}

/** The least and greatest b of the voxels chunk (i, j) of a slice may draw (cellExtent in renderer.ts), found on first use. */
fn cell_extent(of: &FaceView, slice: &mut CachedSlice, index: &LayerIndex, i: i32, j: i32) -> Option<(f64, f64)> {
    *slice.extents.entry((i, j)).or_insert_with(|| {
        let level = slice.id.level;
        let rect = chunk_cell(level, i, j);
        // The cell's sites, in lattice units.
        let (left, right) = ((rect.x / LAYER).round() as i64, ((rect.x + rect.width) / LAYER).round() as i64);
        let (columns, rows) = cell_margin(level);
        // In the units of the grid the slice reads, N times larger.
        let n = 1i64 << atom_level(level);
        let ceil = |v: i64| -(-v).div_euclid(n);
        let (lo, hi) = index.range(
            of.lattice,
            slice.depth.div_euclid(n),
            ceil(slice.depth + slice.size),
            (left - columns as i64).div_euclid(n),
            ceil(right + columns as i64),
        )?;
        let lo = ((lo * n) as f64).max(rect.y - rows as f64);
        let hi = ((hi * n) as f64).min(rect.y + rect.height + rows as f64);
        (lo <= hi).then_some((lo, hi))
    })
}

/**
 * The layer a slice draws, given what nearer layers cover (drawSlice in
 * renderer.ts): chunks it lacks are listed in `missing`, the cell's latest
 * bake standing in.
 */
#[allow(clippy::too_many_arguments)]
fn draw_slice(
    of: &FaceView,
    slice: &mut CachedSlice,
    index: Option<&LayerIndex>,
    mut coverage: Option<&mut Coverage>,
    view: &View,
    fill: bool,
    missing: &mut Vec<(Request, f64, bool)>,
    stats: &mut Stats,
) -> Layer {
    let id = slice.id;
    let camera = of.camera;
    let eye = of.eyes[id.level as usize];
    // Slices start at least NEAR_DEPTH ahead of the camera, so this is > 0.
    let distance = slice_centre(&id) - camera[2];
    let mut layer = Layer { face: of.face, distance, placements: Vec::new() };
    // No grid of the slice's level yet: nothing to draw.
    let Some(index) = index else { return layer };
    let [min_x, max_x, min_y, max_y] = of.tangents;
    let (left, right) = (camera[0] + min_x * distance, camera[0] + max_x * distance);
    let (bottom, top) = (camera[1] + min_y * distance, camera[1] + max_y * distance);
    let reach = cell_margin(id.level).1 as f64;
    let shown = |b: Option<ScreenBox>| b.filter(|b| view.on_screen(b));
    for (i, j) in chunk_cells(id.level, left, right, bottom, top) {
        let rect = chunk_cell(id.level, i, j);
        // Nothing to draw: not baked at all. Otherwise, what it draws lies within reach of its voxels.
        let Some(extent) = cell_extent(of, slice, index, i, j) else { continue };
        let (x0, x1) = (rect.x - camera[0], rect.x + rect.width - camera[0]);
        let y0 = rect.y.max(extent.0 - reach) - camera[1];
        let y1 = (rect.y + rect.height).min(extent.1 + reach) - camera[1];
        if y0 >= y1 || !in_face(closest_to_zero(x0, x1), closest_to_zero(y0, y1), distance) {
            continue;
        }
        let Some(cell_box) = shown(of.boxes.of(x0, x1, y0, y1, distance)) else { continue };
        if coverage.as_ref().is_some_and(|c| c.hides(cell_box.min_x, cell_box.max_x, cell_box.min_y, cell_box.max_y)) {
            stats.covered += 1;
            continue;
        }
        let cell = slice.cells.get(&(i, j));
        let chunk = match cell.and_then(|c| c.find(eye, fill)) {
            Some(chunk) => chunk.clone(),
            None => {
                let latest = cell.and_then(Cell::latest).cloned();
                // Not wanted if what stands in is hidden anyway.
                if let (Some(latest), Some(c)) = (&latest, coverage.as_deref_mut())
                    && latest.baked.painted > 0
                {
                    let handle = known(c, latest);
                    if c.hides_chunk(handle, latest.baked.origin[0] - camera[0], latest.baked.origin[1] - camera[1], distance) {
                        stats.covered += 1;
                        continue;
                    }
                }
                missing.push((Request { face: of.face, slice: id, i, j, eye, fill }, distance, latest.is_some()));
                let Some(latest) = latest else { continue };
                latest
            }
        };
        if chunk.baked.numbers.is_empty() {
            continue;
        }
        // The chunk's content, not just its cell, must reach the screen.
        let (x, y) = (chunk.baked.origin[0] - camera[0], chunk.baked.origin[1] - camera[1]);
        let [width, height] = chunk.baked.extent;
        if shown(of.boxes.of(x, x + width, y - height, y, distance)).is_none() {
            continue;
        }
        if let Some(c) = coverage.as_deref_mut() {
            let handle = known(c, &chunk);
            if c.hides_chunk(handle, x, y, distance) {
                stats.covered += 1;
                continue;
            }
        }
        layer.placements.push(Placement { chunk, x, y });
    }
    layer
}
