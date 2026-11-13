//! Renders the world as chunks that are cells of the lattice (bake's
//! lod.rs): per face, the cells lod.rs picks, relative to the camera,
//! finest and nearest first. A chunk is a whole cell, whatever the camera: baked once the
//! world has all the tiles it reads, it is kept while its face's pick holds
//! it, and a while after; until then, the tiles are what it wants.
//! A frame is every chunk its faces' picks compose for an eye anywhere in
//! them, found as a face is picked and added to as chunks come; which of
//! them the eye draws from where it is, the GPU finds (compositor.wgsl).
//! With it come the last frames' changes, the chunks that came and went,
//! so that the compositor's work follows what changed.
//! See docs/architecture.md, sections 1 to 5.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bake::lod::{CellChunk, Lod, Picked, distance};
use bake::slicing::LAYER;
use bake::{Baker, chunk_reads};
use fcc::Lattice;
use fcc::{FastMap, FastSet, GRID_LEVELS, Tiles};

use super::bakery::{Baked, Bakery, Request};
use crate::engine::Look;
use crate::view::{self, Face, to_local};
use crate::world::{Heights, Level, TILE, TileId, World};

/**
 * Chunks kept beyond the picks, before those not looked up or let go for
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

/** A chunk in a frame: its face and cell. */
#[derive(Clone)]
pub struct Placement {
    pub face: usize,
    pub cell: CellChunk,
    pub chunk: Arc<Chunk>,
}

/**
 * What to draw: chunks face by face, finest and nearest first as they were
 * picked, as the compositor has room for them; and how they came to be.
 */
#[derive(Clone)]
pub struct Frame {
    /** The same from frame to frame until a chunk comes or goes, and then its number. */
    pub chunks: Arc<Vec<Placement>>,
    pub number: u64,
    /** The last changes that led to it, oldest first, each from the frame before. */
    pub changes: Arc<[Arc<Change>]>,
    /** The farthest chunk's distance, or the render distance if farther: the sky's fog. */
    pub farthest: f64,
    /** Whether its chunks' whole voxels are reliefs, or splats each (Renderer's `relief`). */
    pub relief: bool,
    /** Whether its reliefs are drawn flat, a picture a cell (compositor::flatten). */
    pub flat: bool,
}

/**
 * What changed since frame `base`: the chunks that came, in the frame's
 * order, and the ids of those that went. Frames are numbered across all
 * renderers, so that a compositor that drew another's applies none of its
 * changes.
 */
pub struct Change {
    pub base: u64,
    pub came: Vec<Placement>,
    pub went: Vec<u32>,
}

/** Changes a frame keeps: a compositor that missed more takes the frame whole. */
const CHANGES: usize = 64;

/** The next frame's number, of all renderers'. */
fn number() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Frame {
    /** Its changes from frame `number` on, if it keeps them all. */
    pub fn since(&self, number: u64) -> Option<&[Arc<Change>]> {
        let first = self.changes.iter().position(|c| c.base == number)?;
        Some(&self.changes[first..])
    }
}

#[derive(Clone)]
pub struct Stats {
    /** Chunks in view to bake. */
    pub baking: usize,
    /** Tiles that cells in view read and the world has not made, by grid level, most wanted first. */
    pub tiles: Vec<(usize, TileId)>,
}

/**
 * How far the eye may move from where a face's cells were picked, in units
 * of grid 0, and how far its tangents may turn, before they are picked
 * anew: until then, a frame draws what they compose, rather than pick
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

/** A face's cells picked, each with what is known of it, and of those it composes: the chunks, the cells that wait for tiles, and those to bake. */
struct Pick {
    picked: Picked<Seen>,
    chunks: Vec<Placement>,
    waiting: Vec<(CellChunk, [TileId; 2], Wanted)>,
    baking: FastMap<CellChunk, (Request, Wanted)>,
}

/**
 * How far a cell was from the eye when its face was picked, and whether
 * the eye composed it then: the frame waits for those, their tiles asked
 * for; the rest of the pick is baked after, as its tiles come, for the eye
 * to find as it moves.
 */
#[derive(Clone, Copy)]
struct Wanted {
    distance: f64,
    in_view: bool,
}

impl Wanted {
    /** In view first, then nearest first. */
    fn order(&self) -> (bool, f64) {
        (!self.in_view, self.distance)
    }
}

pub struct Renderer {
    faces: Vec<Face>,
    /** Per grid level, the y of its sites, [low, high). */
    heights: Heights,
    /**
     * By face, its cells picked last; and by face and cell, the chunks of
     * those that came and went, or that a bake found empty, and the render
     * that last looked them up or let them go, kept a while: what else was
     * known of a cell is found again at less cost than kept.
     */
    picks: Vec<Option<Pick>>,
    cells: FastMap<(usize, CellChunk), (Seen, u64)>,
    bakery: Bakery,
    next_id: u32,
    renders: u64,
    /** Whether its chunks' whole voxels are reliefs (Request's `relief`). */
    relief: bool,
    /** Whether its reliefs are drawn flat (Request's `flat`). */
    flat: bool,
    /** Whether the chunks it has are lit (Request's `lit`): when the look's lighting changes, they are baked again. */
    lit: bool,
    /** Per level, the world's tiles the cells waiting for tiles were last looked up in. */
    tiles: Vec<Arc<Tiles>>,
    /** The faces in view last, and what their picks composed then: the frame, and what it wants. */
    shown: Vec<bool>,
    frame: Frame,
    stats: Stats,
    /** The frame's chunks' ids, and the changes it keeps. */
    ids: FastSet<u32>,
    changes: VecDeque<Arc<Change>>,
}

/** A cell's voxels' depths along the axis, relative to the camera at `local`, in the face's coordinates: its sites' and a grid unit past. */
fn depths(cell: &CellChunk, local: [f64; 3]) -> [f64; 2] {
    let g = (1i64 << cell.atoms) as f64;
    let half = (cell.spacing() + 1) as f64 * g;
    [-half, half].map(|h| (cell.site[2] as f64 * g + h) * LAYER - local[2])
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
     * bakers on as many threads as the world's, which call `baked` as each
     * chunk is; given light, each can light the voxels it shows (a Light
     * each), as the look asks.
     */
    pub fn new(world: &World, relief: bool, flat: bool, baked: impl Fn() + Send + Sync + 'static) -> Renderer {
        let faces = view::faces();
        let (palette, light) = (world.palette.clone(), world.light());
        let bakery = Bakery::new(world.threads, baked, move || {
            let mut baker = Baker::new(&palette);
            if let Some(light) = &light {
                let mut light = light();
                baker.light = Some(Box::new(move |level, site| light.faces(level as u32, site.map(i64::from))));
            }
            baker
        });
        Renderer {
            picks: faces.iter().map(|_| None).collect(),
            shown: vec![false; faces.len()],
            faces,
            heights: world.heights,
            cells: FastMap::default(),
            bakery,
            next_id: 0,
            renders: 0,
            relief,
            flat,
            lit: false,
            tiles: (0..GRID_LEVELS).map(|_| Arc::new(Tiles::new(0, []))).collect(),
            frame: Frame { chunks: Arc::default(), number: number(), changes: Arc::new([]), farthest: 0.0, relief, flat },
            stats: Stats { baking: 0, tiles: Vec::new() },
            ids: FastSet::default(),
            changes: VecDeque::new(),
        }
    }

    /** The chunks the picks of `look` compose of the world's tiles, each level's in `levels`, and what is missing for the rest. */
    pub fn render(&mut self, look: &Look, levels: &[Arc<Level>]) -> (Frame, &Stats) {
        let mut changed = false;
        if look.sky.is_some() != self.lit {
            self.cells.clear();
            self.picks.iter_mut().for_each(|p| *p = None);
            self.lit = look.sky.is_some();
            changed = true;
        }
        // Cells that lacked tiles can be baked.
        let mut arrived = [false; GRID_LEVELS];
        for (level, arrived) in arrived.iter_mut().enumerate() {
            let tiles = &levels[level].grids;
            *arrived = !Arc::ptr_eq(tiles, &self.tiles[level]);
            if *arrived {
                self.tiles[level] = tiles.clone();
                self.bakery.set_tiles(level, tiles.clone());
            }
        }
        changed |= self.receive();
        self.renders += 1;
        let (camera, view) = (look.camera, look.view());
        let lod = Lod::new(look.shift, look.detail);
        for f in 0..self.faces.len() {
            let tangents = view.face_tangents(&self.faces[f]);
            // Back in view, its cells look for every tile that came meanwhile.
            let back = tangents.is_some() && !self.shown[f];
            changed |= tangents.is_some() != self.shown[f];
            self.shown[f] = tangents.is_some();
            let Some(tangents) = tangents else { continue };
            let local = to_local(&self.faces[f].axes, camera);
            let eye = [local[0] / LAYER, local[1], local[2] / LAYER];
            let key = (lod.shift, lod.q, look.far);
            if self.picks[f].as_ref().is_some_and(|p| p.picked.covers(key, eye, tangents)) {
                changed |= self.look_up(f, if back { &[true; GRID_LEVELS] } else { &arrived }, levels, view.focal, local);
                continue;
            }
            let [t0, t1, t2, t3] = tangents;
            let eyes = eye.map(|v| [v - DRIFT, v + DRIFT]);
            // Within the face's rays: |x| + √2 |y| ≤ z.
            let y = std::f64::consts::FRAC_1_SQRT_2;
            let wide = [(t0 - TURN).max(-1.0), (t1 + TURN).min(1.0), (t2 - TURN).max(-y), (t3 + TURN).min(y)];
            let mut old = self.picks[f].take().map(|p| p.picked);
            let picked = lod.pick(eyes, look.far, wide, old.as_mut());
            // The chunks the new pick does not have are kept a while, as if just looked up.
            let renders = self.renders;
            old.into_iter().for_each(|old| {
                old.into_each(|cell, seen| {
                    if matches!(seen, Seen::Baked(_)) {
                        self.cells.insert((f, cell), (seen, renders));
                    }
                })
            });
            self.compose(f, picked, tangents, &lod, look.far, levels, view.focal, local);
            changed = true;
        }
        if changed {
            self.gather(look);
        }
        self.evict();
        (self.frame.clone(), &self.stats)
    }

    /**
     * What face `f`'s `picked` composes, the eye at `local` in its
     * coordinates, its rays within `tangents`, found finest first, and
     * within a level layer by layer from the nearest (Lod::composed): what
     * the compositor takes first.
     */
    #[allow(clippy::too_many_arguments)]
    fn compose(
        &mut self,
        f: usize,
        mut picked: Picked<Seen>,
        tangents: [f64; 4],
        lod: &Lod,
        far: f64,
        levels: &[Arc<Level>],
        focal: f64,
        local: [f64; 3],
    ) {
        let (lattice, request) = (self.faces[f].lattice, self.request(f, focal, local));
        let eye = [local[0] / LAYER, local[1], local[2] / LAYER];
        let (mut chunks, mut waiting, mut baking) = (Vec::new(), Vec::new(), FastMap::default());
        let (eyes, wide) = picked.bounds();
        lod.composed(&mut picked, eyes, wide, |cell, seen| {
            if matches!(seen, Seen::Empty) {
                return;
            }
            let level = cell.atoms as usize;
            // What is known, brought up to date: its chunk kept, or what tiles it reads; then as its tiles come
            // (look_up) and its chunk (receive).
            if matches!(seen, Seen::Unknown) {
                *seen = match self.cells.get_mut(&(f, cell)) {
                    Some(known) => {
                        known.1 = self.renders;
                        known.0.clone()
                    }
                    None => {
                        let (_, min, max) = chunk_reads(&cell);
                        box_tiles(lattice, self.heights[level], min, max).map_or(Seen::Empty, Seen::Tiles)
                    }
                };
            }
            let source = &levels[level];
            if let Seen::Tiles(t) = *seen
                && source.grids.has_all(t)
            {
                *seen = if may_hold(&cell, lattice.f, source, t) { Seen::Bake(t) } else { Seen::Empty };
            }
            let wanted = Wanted { distance: distance(&cell, eye), in_view: lod.composes(&cell, eye, far, tangents) };
            match seen {
                Seen::Unknown | Seen::Empty => (),
                &mut Seen::Tiles(t) => waiting.push((cell, t, wanted)),
                &mut Seen::Bake(t) => _ = baking.insert(cell, (request(cell, t), wanted)),
                Seen::Baked(chunk) => chunks.push(Placement { face: f, cell, chunk: chunk.clone() }),
            }
        });
        self.picks[f] = Some(Pick { picked, chunks, waiting, baking });
    }

    /** Requests to bake a cell of face `f` that reads given tiles, the eye at `local` seeing at `focal`. */
    fn request(&self, f: usize, focal: f64, local: [f64; 3]) -> impl Fn(CellChunk, [TileId; 2]) -> Request + use<> {
        let (lit, relief, flat) = (self.lit, self.relief, self.flat);
        move |cell, tiles| {
            let block = super::compositor::block(depths(&cell, local)[1].max(1.0), focal);
            Request { face: f, cell, tiles, block, lit, relief, flat }
        }
    }

    /** Of face `f`'s cells that wait for tiles, those whose tiles arrived (by level, `arrived`) are to bake, or hold nothing. Whether any were. */
    fn look_up(&mut self, f: usize, arrived: &[bool; GRID_LEVELS], levels: &[Arc<Level>], focal: f64, local: [f64; 3]) -> bool {
        let Some(mut pick) = self.picks[f].take() else { return false };
        let (lattice, request) = (self.faces[f].lattice, self.request(f, focal, local));
        let before = pick.waiting.len();
        pick.waiting.retain(|&(cell, t, wanted)| {
            let level = cell.atoms as usize;
            let source = &levels[level];
            if !arrived[level] || !source.grids.has_all(t) {
                return true;
            }
            let seen = if may_hold(&cell, lattice.f, source, t) {
                pick.baking.insert(cell, (request(cell, t), wanted));
                Seen::Bake(t)
            } else {
                Seen::Empty
            };
            if let Some(picked) = pick.picked.at(&cell) {
                *picked = seen;
            }
            false
        });
        let changed = pick.waiting.len() < before;
        self.picks[f] = Some(pick);
        changed
    }

    /** The frame of what the faces in view composed, and what they want: the chunks to bake, and the tiles, nearest first. */
    fn gather(&mut self, look: &Look) {
        let picks = || self.picks.iter().zip(&self.shown).filter(|p| *p.1).filter_map(|p| p.0.as_ref());
        let chunks: Vec<Placement> = picks().flat_map(|p| p.chunks.iter().cloned()).collect();
        // Their planes through their sites.
        let farthest = chunks
            .iter()
            .map(|p| {
                let [near, far] = depths(&p.cell, to_local(&self.faces[p.face].axes, look.camera));
                (near + far) / 2.0
            })
            .fold(look.far, f64::max);
        self.frame.farthest = farthest;
        let ids: FastSet<u32> = chunks.iter().map(|p| p.chunk.id).collect();
        let came: Vec<Placement> = chunks.iter().filter(|p| !self.ids.contains(&p.chunk.id)).cloned().collect();
        let went: Vec<u32> = self.ids.iter().filter(|id| !ids.contains(id)).copied().collect();
        if !came.is_empty() || !went.is_empty() {
            if self.changes.len() == CHANGES {
                self.changes.pop_front();
            }
            self.changes.push_back(Arc::new(Change { base: self.frame.number, came, went }));
            (self.frame.chunks, self.frame.number) = (Arc::new(chunks), number());
            self.frame.changes = self.changes.iter().cloned().collect();
            self.ids = ids;
        }
        let mut baking: Vec<&(Request, Wanted)> = picks().flat_map(|p| p.baking.values()).collect();
        baking.sort_by(|a, b| a.1.order().partial_cmp(&b.1.order()).expect("distances"));
        let wanted: Vec<Request> = baking.iter().map(|b| b.0).collect();
        self.bakery.want(&wanted);
        let mut tiles: Vec<(usize, TileId, f64)> = picks()
            .flat_map(|p| &p.waiting)
            .filter(|w| w.2.in_view)
            .flat_map(|&(cell, t, wanted)| {
                let (level, source) = (cell.atoms as usize, &self.tiles[cell.atoms as usize]);
                tiles_in(t).filter(|&id| !source.has(id)).map(move |id| (level, id, wanted.distance))
            })
            .collect();
        tiles.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut listed = FastSet::default();
        self.stats = Stats {
            baking: baking.iter().filter(|b| b.1.in_view).count(),
            tiles: tiles.into_iter().filter(|t| listed.insert((t.0, t.1))).map(|t| (t.0, t.1)).collect(),
        };
    }

    /** Keeps what the bakery delivered, where its face's pick has its cell, and a while for when it comes again. Whether any came. */
    fn receive(&mut self) -> bool {
        let mut any = false;
        for (r, baked) in self.bakery.take() {
            // Baked as it was before it changed.
            if r.lit != self.lit {
                continue;
            }
            self.next_id += 1;
            let seen = if baked.is_empty() { Seen::Empty } else { Seen::Baked(Arc::new(Chunk { id: self.next_id, baked })) };
            if let Some(pick) = self.picks[r.face].as_mut()
                && let Some(picked) = pick.picked.at(&r.cell)
            {
                *picked = seen.clone();
                if pick.baking.remove(&r.cell).is_some() {
                    if let Seen::Baked(chunk) = &seen {
                        pick.chunks.push(Placement { face: r.face, cell: r.cell, chunk: chunk.clone() });
                    }
                    any = true;
                }
            }
            self.cells.insert((r.face, r.cell), (seen, self.renders));
        }
        any
    }

    /** Forgets cells not looked up or let go a while: the picks keep what they know of theirs. */
    fn evict(&mut self) {
        if self.cells.len() > MAX_CELLS && self.renders.is_multiple_of(EVICTS) {
            let renders = self.renders;
            self.cells.retain(|_, (_, used)| *used + UNUSED_RENDERS > renders);
        }
    }
}
