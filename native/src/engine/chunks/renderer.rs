//! Renders the world as chunks that are cells of the lattice (bake's
//! lod.rs): per face, the cells lod.rs picks, relative to the camera,
//! finest and nearest first. A chunk is a whole cell, whatever the camera: baked once the
//! world has all the tiles it reads, it is kept while its face's pick holds
//! it, and a while after; until then, the tiles are what it wants. Every chunk in view is baked: drawing what
//! is in view, and culling what of it is hidden, is the compositor's job.
//! See docs/architecture.md, sections 1 to 5.

use std::collections::HashMap;
use std::sync::Arc;

use bake::lod::{CellChunk, Lod, Picked, distance};
use bake::slicing::{GRID_LEVELS, LAYER};
use bake::{Baker, Lattice, chunk_reads};

use super::bakery::{Baked, Bakery, Request};
use crate::engine::Look;
use crate::view::{self, Face, in_face, to_local};
use crate::world::{Heights, Level, TILE, TileId, World};

/**
 * Cells known beyond the picks, before those not looked up or let go for
 * UNUSED_RENDERS are forgotten, looked for every EVICTS renders: each
 * time, all are.
 */
const MAX_CELLS: usize = 100_000;
const UNUSED_RENDERS: u64 = 300;
const EVICTS: u64 = 60;

/** A baked chunk. `id` names it to the compositor. */
pub struct Chunk {
    pub id: u32,
    pub baked: Baked,
}

/**
 * A chunk in a frame: its face and cell, its plane `distance` ahead of the
 * camera along the face's axis, and the top-left corner of what it draws
 * there, relative to the camera, in the face's coordinates; how far its
 * cell's site is from the camera, and the last layer of grid 0 whose
 * cells the next finer level draws (Lod::finer_reach).
 */
pub struct Placement {
    pub face: usize,
    pub cell: CellChunk,
    pub distance: f64,
    pub chunk: Arc<Chunk>,
    pub x: f64,
    pub y: f64,
    pub from_eye: f64,
    pub finer: Option<i64>,
}

/** What to draw: chunks face by face, finest and nearest first, as the compositor has room for them. */
pub struct Frame {
    pub chunks: Vec<Placement>,
    /** The farthest chunk's distance, or the render distance if farther: the sky's fog. */
    pub farthest: f64,
    /** Whether its chunks' whole voxels are reliefs, or splats each (Renderer's `relief`). */
    pub relief: bool,
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

/**
 * How far the eye may move from where a face's cells were picked, in units
 * of grid 0, and how far its tangents may turn, before they are picked
 * anew: each frame finds which of them it composes now, rather than pick
 * hundreds of thousands of cells, and knows what it knew of them.
 */
const DRIFT: f64 = 8.0;
const TURN: f64 = 0.15;

/**
 * What is known of a cell of a face: nothing yet; that it holds nothing
 * (all above or below the world, outside its grid's index, or baked
 * empty); that it waits for the tiles it reads (box_tiles), or for its
 * bake; or its chunk.
 */
#[derive(Clone, Default)]
enum Seen {
    #[default]
    Unknown,
    Empty,
    Tiles([TileId; 2]),
    Bake([TileId; 2]),
    Baked(Arc<Chunk>),
}

pub struct Renderer {
    faces: Vec<Face>,
    /** Per grid level, the y of its sites, [low, high). */
    heights: Heights,
    /**
     * By face, its cells picked last, each with what is known of it; and
     * by face and cell, what was known of those that came and went, and the
     * render that last looked them up or let them go, kept a while.
     */
    picks: Vec<Option<Picked<Seen>>>,
    cells: HashMap<(usize, CellChunk), (Seen, u64)>,
    bakery: Bakery,
    next_id: u32,
    renders: u64,
    /** Whether its chunks' whole voxels are reliefs (Request's `relief`). */
    relief: bool,
    /** Whether the chunks it has are lit (Request's `lit`): when the look's lighting changes, they are baked again. */
    lit: bool,
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
 * The tiles a box of (a, b, c) of a face's lattice coordinates meets
 * within the world, its sites' y in [low, high) of `heights`, in a grid's
 * units: the first and the last, each way; None if it is all above or
 * below the world.
 */
fn box_tiles(Lattice { r, u, f }: Lattice, heights: (i64, i64), min: [i32; 3], max: [i32; 3]) -> Option<[TileId; 2]> {
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
    (lo[1], hi[1]) = (lo[1].max(heights.0 as f64), hi[1].min((heights.1 - 1) as f64));
    let tile = |v: f64| (v / TILE as f64).floor() as i32;
    Some([lo.map(tile), hi.map(tile)])
}

/** The tiles from `lo` to `hi`, inclusive. */
fn tiles_in([lo, hi]: [TileId; 2]) -> impl Iterator<Item = TileId> {
    (lo[2]..=hi[2]).flat_map(move |k| (lo[1]..=hi[1]).flat_map(move |j| (lo[0]..=hi[0]).map(move |i| [i, j, k])))
}

/** Whether a cell may hold voxels: its grid's index has some in its layers and columns, within its rows. Once it has its tiles. */
fn may_hold(cell: &CellChunk, forward: [i32; 3], level: &Level, tiles: [TileId; 2]) -> bool {
    let (min, max) = cell.bounds();
    let (c0, c1, a0, a1) = (min[2] as i64, max[2] as i64 + 1, min[0] as i64, max[0] as i64 + 1);
    tiles_in(tiles)
        .filter_map(|t| level.tiles.get(&t)?.index.range(forward, c0, c1, a0, a1))
        .any(|(lo, hi)| lo <= max[1] as i64 && hi >= min[1] as i64)
}

impl Renderer {
    /**
     * A renderer of `world`, its whole voxels reliefs or splats each, its
     * bakers on as many threads as the world's; given light, each can light
     * the voxels it shows (a Light each), as the look asks.
     */
    pub fn new(world: &World, relief: bool) -> Renderer {
        let faces = view::faces();
        let (palette, light) = (world.palette.clone(), world.light());
        let bakery = Bakery::new(world.threads, move || {
            let mut baker = Baker::new(&palette);
            if let Some(light) = &light {
                let mut light = light();
                baker.light = Some(Box::new(move |level, site| light.faces(level as u32, site.map(i64::from))));
            }
            baker
        });
        Renderer {
            picks: faces.iter().map(|_| None).collect(),
            faces,
            heights: world.heights,
            cells: HashMap::new(),
            bakery,
            next_id: 0,
            renders: 0,
            relief,
            lit: false,
        }
    }

    /** Waits until the bakery delivers a chunk. */
    pub fn wait(&mut self) {
        self.bakery.wait();
    }

    /** The chunks in view of `look` of `world`'s tiles, and what is missing for the rest. */
    pub fn render(&mut self, look: &Look, world: &World) -> (Frame, Stats) {
        if look.sky.is_some() != self.lit {
            self.cells.clear();
            self.picks.iter_mut().for_each(|p| *p = None);
            self.lit = look.sky.is_some();
        }
        // Cells that lacked tiles can be baked.
        for level in 0..GRID_LEVELS {
            self.bakery.set_tiles(level, world.level(level).grids.clone());
        }
        self.receive();
        self.renders += 1;
        let (camera, view) = (look.camera, look.view());
        let lod = Lod::new(look.shift, look.detail);
        let mut frame = Frame { chunks: Vec::new(), farthest: look.far, relief: self.relief };
        let mut stats = Stats::default();
        let mut missing: Vec<(Request, f64)> = Vec::new();
        let mut tiles: Vec<(usize, TileId, f64)> = Vec::new();
        for f in 0..self.faces.len() {
            let face = &self.faces[f];
            let Some(tangents) = view.face_tangents(face) else { continue };
            let lattice = face.lattice;
            let local = to_local(&face.axes, camera);
            let boxes = view.screen_boxes(face);
            let eye = [local[0] / LAYER, local[1], local[2] / LAYER];
            let far = look.far;
            let key = (lod.shift, lod.q, far);
            let pick = &mut self.picks[f];
            if !pick.as_ref().is_some_and(|p| p.covers(key, eye, tangents)) {
                let [t0, t1, t2, t3] = tangents;
                let eyes = eye.map(|v| [v - DRIFT, v + DRIFT]);
                // Within the face's rays: |x| + √2 |y| ≤ z.
                let y = std::f64::consts::FRAC_1_SQRT_2;
                let wide = [(t0 - TURN).max(-1.0), (t1 + TURN).min(1.0), (t2 - TURN).max(-y), (t3 + TURN).min(y)];
                let mut old = pick.take();
                *pick = Some(lod.pick(eyes, far, wide, old.as_mut()));
                // What the new pick does not have is kept a while, as if just looked up.
                let renders = self.renders;
                old.into_iter().for_each(|old| {
                    old.into_each(|cell, seen| {
                        if !matches!(seen, Seen::Unknown) {
                            self.cells.insert((f, cell), (seen, renders));
                        }
                    })
                });
            }
            // What it draws comes finest first, and within a level layer by layer from the nearest (Lod::composed):
            // what the compositor takes first.
            lod.composed(pick.as_mut().expect("picked"), eye, tangents, |cell, seen| {
                if matches!(seen, Seen::Empty) {
                    return;
                }
                let level = cell.atoms as usize;
                let g = (1i64 << cell.atoms) as f64;
                // Its box, relative to the camera, in the face's coordinates.
                let half = (cell.spacing() as f64) * g;
                let [a, b, c] = cell.site.map(|v| v as f64 * g);
                let (x0, x1) = ((a - half) * LAYER - local[0], (a + half) * LAYER - local[0]);
                let (y0, y1) = (b - half - local[1], b + half - local[1]);
                let (z0, z1) = ((c - half) * LAYER - local[2], (c + half) * LAYER - local[2]);
                if !in_face(closest_to_zero(x0, x1), closest_to_zero(y0, y1), z1) {
                    return;
                }
                let on_screen = |z: f64| boxes.of(x0, x1, y0, y1, z).is_some_and(|b| view.on_screen(&b));
                if !on_screen(z0.max(1e-3)) && !on_screen(z1) {
                    return;
                }
                // What is known, brought up to date: looked up once, then as its tiles come (its chunk, receive).
                let source = world.level(level);
                if matches!(seen, Seen::Unknown) {
                    let known = self.cells.entry((f, cell)).or_default();
                    if matches!(known.0, Seen::Unknown) {
                        let (_, min, max) = chunk_reads(&cell);
                        known.0 = box_tiles(lattice, self.heights[level], min, max).map_or(Seen::Empty, Seen::Tiles);
                    }
                    (*seen, known.1) = (known.0.clone(), self.renders);
                }
                if let Seen::Tiles(t) = *seen
                    && source.grids.has_all(t)
                {
                    *seen = if may_hold(&cell, lattice.f, source, t) { Seen::Bake(t) } else { Seen::Empty };
                    self.cells.insert((f, cell), (seen.clone(), self.renders));
                }
                let d = distance(&cell, eye);
                match seen {
                    Seen::Unknown | Seen::Empty => return,
                    &mut Seen::Tiles(t) => {
                        tiles.extend(tiles_in(t).filter(|&id| !source.grids.has(id)).map(|id| (level, id, d)));
                    }
                    &mut Seen::Bake(t) => {
                        let block = super::compositor::block(z1.max(1.0), view.focal);
                        missing.push((Request { face: f, cell, tiles: t, block, lit: self.lit, relief: self.relief }, d));
                    }
                    Seen::Baked(chunk) => {
                        // Its plane, through its site.
                        let distance = c * LAYER - local[2];
                        let (x, y) = (chunk.baked.origin[0] - local[0], chunk.baked.origin[1] - local[1]);
                        let [width, height] = chunk.baked.extent;
                        if distance > 0.0 && boxes.of(x, x + width, y - height, y, distance).is_some_and(|b| view.on_screen(&b)) {
                            frame.farthest = frame.farthest.max(distance);
                            let finer = lod.finer_reach(cell.level, eye[2]);
                            frame.chunks.push(Placement { face: f, cell, distance, chunk: chunk.clone(), x, y, from_eye: d, finer });
                        }
                    }
                }
                stats.levels[level] = true;
            });
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

    /** Keeps what the bakery delivered, where its face's pick has its cell, and a while for when it comes again. */
    fn receive(&mut self) {
        for (r, baked) in self.bakery.take() {
            // Baked as it was before it changed.
            if r.lit != self.lit {
                continue;
            }
            self.next_id += 1;
            let seen = if baked.is_empty() { Seen::Empty } else { Seen::Baked(Arc::new(Chunk { id: self.next_id, baked })) };
            if let Some(picked) = self.picks[r.face].as_mut().and_then(|p| p.at(&r.cell)) {
                *picked = seen.clone();
            }
            self.cells.insert((r.face, r.cell), (seen, self.renders));
        }
    }

    /** Forgets cells not looked up or let go a while: the picks keep what they know of theirs. */
    fn evict(&mut self) {
        if self.cells.len() > MAX_CELLS && self.renders.is_multiple_of(EVICTS) {
            let renders = self.renders;
            self.cells.retain(|_, (_, used)| *used + UNUSED_RENDERS > renders);
        }
    }
}
