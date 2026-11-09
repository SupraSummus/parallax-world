//! Renders the world as chunks that are cells of the lattice (bake's
//! lod.rs): per face, the cells lod.rs picks, relative to the camera,
//! finest and nearest first. A chunk is a whole cell, whatever the camera: baked once the
//! world has all the tiles it reads, it is kept while in use; until then,
//! the tiles are what it wants. Every chunk in view is baked: drawing what
//! is in view, and culling what of it is hidden, is the compositor's job.
//! See docs/architecture.md, sections 1 to 5.

use std::collections::HashMap;
use std::sync::Arc;

use bake::lod::{CellChunk, Lod};
use bake::slicing::{GRID_LEVELS, LAYER};
use bake::{Lattice, chunk_reads};

use crate::bakery::{Baked, Bakery, Request};
use crate::view::{Face, Vec3, View, in_face, to_local};
use crate::world::{Level, TILE, TileId};

/** Cells kept, in use or not, before those unused for UNUSED_RENDERS are dropped. */
const MAX_CELLS: usize = 100_000;
const UNUSED_RENDERS: u64 = 300;

/** A baked chunk. `id` names it to the compositor. */
pub struct Chunk {
    pub id: u32,
    pub baked: Baked,
}

/**
 * A chunk in a frame: its face and cell, its plane `distance` ahead of the
 * camera along the face's axis, and the top-left corner of what it draws
 * there, relative to the camera, in the face's coordinates.
 */
pub struct Placement {
    pub face: usize,
    pub cell: CellChunk,
    pub distance: f64,
    pub chunk: Arc<Chunk>,
    pub x: f64,
    pub y: f64,
}

/** What to draw: chunks face by face, finest and nearest first, as the compositor has room for them. */
pub struct Frame {
    pub chunks: Vec<Placement>,
    /** The farthest chunk's distance, or the render distance if farther: the sky's fog. */
    pub farthest: f64,
}

#[derive(Default)]
pub struct Stats {
    /** Chunks in view to bake, nearest first. */
    pub wanted: Vec<Request>,
    /** The grid levels the frame's chunks read, whether they have tiles yet or not. */
    pub levels: [bool; GRID_LEVELS],
    /** Tiles that cells in view read and the world has not made, by grid level, most wanted first. */
    pub tiles: Vec<(usize, TileId)>,
}

/** What is known of a cell of a face: the tiles it reads (None if all above or below the world); once it has them, whether it may hold voxels; its chunk, once baked; and the render that last used it. */
struct Cell {
    tiles: Option<[i32; 4]>,
    holds: Option<bool>,
    chunk: Option<Arc<Chunk>>,
    used: u64,
}

pub struct Renderer {
    faces: Vec<Face>,
    /** Per grid level, its tiles made so far, and the y of its sites, [low, high). */
    levels: Vec<Arc<Level>>,
    heights: [(i64, i64); GRID_LEVELS],
    /** By face and chunk: what is known of its cell. */
    cells: HashMap<(usize, CellChunk), Cell>,
    bakery: Bakery,
    next_id: u32,
    renders: u64,
    /** Viewing distance over a cell's thickness (Lod::new). */
    pub detail: f64,
    pub render_distance: f64,
    /** How many levels finer than its cell a chunk's voxels are (Lod). */
    pub shift: i32,
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

/**
 * The tiles a box of (a, b, c) of a face's lattice coordinates meets,
 * [i0, k0, i1, k1], in a grid's units; None if it is all above or below
 * the world, its sites' y in [low, high) of `heights`.
 */
fn box_tiles(Lattice { r, u, f }: Lattice, heights: (i64, i64), min: [i32; 3], max: [i32; 3]) -> Option<[i32; 4]> {
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
}

/** Whether a cell may hold voxels: its grid's index has some in its layers and columns, within its rows. Once it has its tiles. */
fn may_hold(cell: &CellChunk, forward: [i32; 3], level: &Level, [i0, k0, i1, k1]: [i32; 4]) -> bool {
    let (min, max) = cell.bounds();
    let (c0, c1, a0, a1) = (min[2] as i64, max[2] as i64 + 1, min[0] as i64, max[0] as i64 + 1);
    (k0..=k1)
        .flat_map(|k| (i0..=i1).map(move |i| (i, k)))
        .filter_map(|t| level.tiles.get(&t)?.index.range(forward, c0, c1, a0, a1))
        .any(|(lo, hi)| lo <= max[1] as i64 && hi >= min[1] as i64)
}

impl Renderer {
    /** A renderer of a world whose sites of each grid level have y in [low, high) of `heights`. */
    pub fn new(faces: Vec<Face>, bakery: Bakery, render_distance: f64, heights: [(i64, i64); GRID_LEVELS]) -> Renderer {
        Renderer {
            faces,
            levels: (0..GRID_LEVELS).map(|_| Arc::new(Level::new(HashMap::new()))).collect(),
            heights,
            cells: HashMap::new(),
            bakery,
            next_id: 0,
            renders: 0,
            // DEFAULT_DETAIL in renderer.ts.
            detail: 2f64.powf(3.25),
            render_distance,
            shift: 3,
        }
    }

    /** Draws the tiles of grid level `level` from now on: cells that lacked them can be baked. */
    pub fn set_tiles(&mut self, level: usize, tiles: Arc<Level>) {
        self.bakery.set_tiles(level, tiles.grids.clone());
        self.levels[level] = tiles;
    }

    /** The tiles of `level` it has. */
    pub fn level(&self, level: usize) -> &Level {
        &self.levels[level]
    }

    /** Waits until the bakery delivers a chunk. */
    pub fn wait(&mut self) {
        self.bakery.wait();
    }

    pub fn render(&mut self, camera: Vec3, view: &View) -> (Frame, Stats) {
        self.receive();
        self.renders += 1;
        let lod = Lod::new(self.shift, self.detail);
        let mut frame = Frame { chunks: Vec::new(), farthest: self.render_distance };
        let mut stats = Stats::default();
        let mut missing: Vec<(Request, f64)> = Vec::new();
        let mut tiles: Vec<(usize, TileId, f64)> = Vec::new();
        for f in 0..self.faces.len() {
            let face = &self.faces[f];
            let Some(tangents) = view.face_tangents(face) else { continue };
            let lattice = face.lattice;
            let local = to_local(&face.axes, camera);
            let boxes = view.screen_boxes(face);
            let mut picked = Vec::new();
            let eye = [local[0] / LAYER, local[1], local[2] / LAYER];
            lod.select(eye, self.render_distance, tangents, |cell, d| picked.push((cell, d)));
            // Finest and nearest first: what the compositor takes first.
            picked.sort_by(|a, b| (a.0.level, a.1).partial_cmp(&(b.0.level, b.1)).expect("distances"));
            for (cell, d) in picked {
                let level = cell.atoms as usize;
                let g = (1i64 << cell.atoms) as f64;
                // Its box, relative to the camera, in the face's coordinates.
                let half = (cell.spacing() as f64) * g;
                let [a, b, c] = cell.site.map(|v| v as f64 * g);
                let (x0, x1) = ((a - half) * LAYER - local[0], (a + half) * LAYER - local[0]);
                let (y0, y1) = (b - half - local[1], b + half - local[1]);
                let (z0, z1) = ((c - half) * LAYER - local[2], (c + half) * LAYER - local[2]);
                if !in_face(closest_to_zero(x0, x1), closest_to_zero(y0, y1), z1) {
                    continue;
                }
                let on_screen = |z: f64| boxes.of(x0, x1, y0, y1, z).is_some_and(|b| view.on_screen(&b));
                if !on_screen(z0.max(1e-3)) && !on_screen(z1) {
                    continue;
                }
                let heights = self.heights[level];
                let known = self.cells.entry((f, cell)).or_insert_with(|| {
                    let (_, min, max) = chunk_reads(&cell);
                    Cell { tiles: box_tiles(lattice, heights, min, max), holds: None, chunk: None, used: 0 }
                });
                known.used = self.renders;
                let Some(t) = known.tiles else { continue };
                if known.holds == Some(false) {
                    continue;
                }
                stats.levels[level] = true;
                let source = &self.levels[level];
                if known.holds.is_none() {
                    if !source.grids.has_all(t) {
                        for k in t[1]..=t[3] {
                            for i in t[0]..=t[2] {
                                if !source.grids.has(i, k) {
                                    tiles.push((level, (i, k), d));
                                }
                            }
                        }
                        continue;
                    }
                    known.holds = Some(may_hold(&cell, lattice.f, source, t));
                    if known.holds == Some(false) {
                        continue;
                    }
                }
                let Some(chunk) = known.chunk.clone() else {
                    let block = crate::compositor::block(z1.max(1.0), view.focal);
                    missing.push((Request { face: f, cell, tiles: t, block }, d));
                    continue;
                };
                if chunk.baked.splats.is_empty() {
                    continue;
                }
                // Its plane, through its site.
                let distance = c * LAYER - local[2];
                let (x, y) = (chunk.baked.origin[0] - local[0], chunk.baked.origin[1] - local[1]);
                let [width, height] = chunk.baked.extent;
                if distance <= 0.0 || !boxes.of(x, x + width, y - height, y, distance).is_some_and(|b| view.on_screen(&b)) {
                    continue;
                }
                frame.farthest = frame.farthest.max(distance);
                frame.chunks.push(Placement { face: f, cell, distance, chunk, x, y });
            }
        }
        missing.sort_by(|a, b| a.1.total_cmp(&b.1));
        stats.wanted = missing.into_iter().map(|m| m.0).collect();
        tiles.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut listed = std::collections::HashSet::new();
        stats.tiles = tiles.into_iter().filter(|t| listed.insert((t.0, t.1))).map(|t| (t.0, t.1)).collect();
        self.bakery.want(&stats.wanted);
        self.evict();
        (frame, stats)
    }

    /** Keeps what the bakery delivered, unless its cell was dropped meanwhile. */
    fn receive(&mut self) {
        for (r, baked) in self.bakery.take() {
            if let Some(known) = self.cells.get_mut(&(r.face, r.cell)) {
                self.next_id += 1;
                known.chunk = Some(Arc::new(Chunk { id: self.next_id, baked }));
            }
        }
    }

    fn evict(&mut self) {
        if self.cells.len() > MAX_CELLS {
            let renders = self.renders;
            self.cells.retain(|_, cell| cell.used + UNUSED_RENDERS > renders);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{focal_length, view_basis};
    use crate::world::World;

    #[test]
    fn moving_keeps_most_chunks() {
        let make = worlds::by_name("moor").expect("moor").make;
        let shown = make(1);
        let mut renderer = crate::renderer(&shown, None, 4, 400.0);
        let mut world = World::new(make, 1, None, 4);
        let [width, height] = [320.0, 200.0];
        let view = View { yaw: 30.0, pitch: -10.0, width, height, focal: focal_length(90.0, width, height) };
        let camera = shown.spawn.map(|v| v as f64);
        let ids = |frame: &Frame| frame.chunks.iter().map(|p| p.chunk.id).collect::<std::collections::HashSet<_>>();
        let (frame, _) = crate::complete(&mut world, &mut renderer, camera, &view, false);
        let before = ids(&frame);
        // A few units along the view: each face's levels claim cells a little farther on.
        let forward = view_basis(view.yaw, view.pitch)[2];
        let camera = [0, 1, 2].map(|i| camera[i] + forward[i] * 7.0);
        let (frame, _) = crate::complete(&mut world, &mut renderer, camera, &view, false);
        let after = ids(&frame);
        let kept = after.intersection(&before).count();
        // Level 0's cells, which change most, are half the frame's in the moor's heather, seen through.
        assert!(kept * 10 > after.len() * 7, "{kept} of {} kept", after.len());
    }
}
