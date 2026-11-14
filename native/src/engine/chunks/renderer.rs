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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bake::lod::{CellChunk, Lod, Picked, distance};
use bake::slicing::LAYER;
use bake::{Baker, chunk_reads, medium_reads};
use fcc::Lattice;
use fcc::{FastMap, FastSet, GRID_LEVELS, Tiles};

use super::bakery::{Baked, Bakery, Request};
use super::compositor::{Flat, probe};
use super::occlusion::{Box3, Occlusion};
use crate::engine::Look;
use crate::view::{self, Face, to_local};
use crate::world::{Heights, Level, TILE, TileId, World, within};

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
    /** Where the compositor tells which of its probes show, and how many times it had told when this frame was rendered. */
    pub feedback: Arc<Feedback>,
    pub heard: u64,
}

/**
 * What the compositor found of the frames' probes (compositor::probe): the
 * chunk ids of those that show, and how many times it found any; and how
 * to wake the renderer, so that it bakes their cells.
 */
pub struct Feedback {
    found: Mutex<(Vec<u32>, u64)>,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Feedback {
    /** Tells the renderer that the probes of chunk ids `ids` show, if any do. */
    pub fn report(&self, ids: Vec<u32>) {
        if ids.is_empty() {
            return;
        }
        {
            let mut found = self.found.lock().expect("feedback");
            found.0.extend(ids);
            found.1 += 1;
        }
        (self.wake)();
    }

    /** How many times it was told of probes that show. */
    pub fn reports(&self) -> u64 {
        self.found.lock().expect("feedback").1
    }

    /** The ids it was told of since the last call, and how many times it was told in all. */
    fn take(&self) -> (Vec<u32>, u64) {
        let mut found = self.found.lock().expect("feedback");
        (std::mem::take(&mut found.0), found.1)
    }
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
    /** Chunks to bake, in view or not. */
    pub queued: usize,
    /** Tiles that cells in view read and the world has not made, by grid level, most wanted first. */
    pub tiles: Vec<(usize, TileId)>,
    /** Cells hidden in the faces in view (Hiding), each a probe among the frame's chunks, and how many the GPU found showing after all, of all hidden. */
    pub hidden: usize,
    pub found: usize,
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
 * (all above or below the world, meeting none of its level's voxels,
 * or baked empty); that it is hidden (Hiding), so not baked while that
 * holds; that it waits for the tiles it reads (box_tiles), or for its
 * bake; or its chunk.
 */
#[derive(Clone, Default)]
enum Seen {
    #[default]
    Unknown,
    Empty,
    Hidden,
    Tiles([TileId; 2]),
    Bake([TileId; 2]),
    Baked(Arc<Chunk>),
}

/**
 * A face's cells picked, each with what is known of it, and of those it
 * composes: the chunks, the cells that wait for tiles, and those to bake;
 * and, of whole voxels, what hides cells, and those it hid.
 */
struct Pick {
    picked: Picked<Seen>,
    chunks: Vec<Placement>,
    waiting: Vec<(CellChunk, [TileId; 2], Wanted)>,
    baking: FastMap<CellChunk, (Request, Wanted)>,
    hiding: Option<Hiding>,
    hidden: Vec<Hidden>,
    /** The cells the GPU found showing after all: not hidden again while picked. */
    showing: FastSet<CellChunk>,
}

/**
 * A cell hidden (Hiding): the tiles it reads, how it is wanted, its request
 * if it was to bake, and its probe, which asks the GPU whether it shows
 * after all (compositor::probe).
 */
struct Hidden {
    cell: CellChunk,
    tiles: [TileId; 2],
    wanted: Wanted,
    request: Option<Request>,
    probe: Arc<Chunk>,
}

/** The probes of face `face`'s hidden cells: their chunks' ids, from the renderer's, and by id, each's face and cell. */
struct Asking<'a> {
    face: usize,
    next_id: &'a mut u32,
    probes: &'a mut FastMap<u32, (usize, CellChunk)>,
}

impl Asking<'_> {
    /** A probe of `cell`, whose box is `shown`. */
    fn probe(&mut self, cell: CellChunk, shown: Box3) -> Arc<Chunk> {
        *self.next_id += 1;
        let (origin, clusters) = probe(shown, f64::from(cell.site[2] << cell.atoms) * LAYER);
        let baked = Baked { origin, columns: Vec::new(), splats: Vec::new(), clusters, veils: 1, flat: Flat::default() };
        self.probes.insert(*self.next_id, (self.face, cell));
        Arc::new(Chunk { id: *self.next_id, baked })
    }
}

/**
 * What hides a face's cells (occlusion.rs): rays cast from where the eye
 * was when the face was picked through the voxels drawn along them, the grid of the level claiming
 * each layer (Lod::claiming), each stopped at the first whole voxel it
 * meets. So ground, walls, trunks and leaves (shown as grain, as often as
 * they cover: worldgen's VoxelGrid::show) hide what is behind them, and a
 * wood what is deep enough in it; see-through voxels, mist, hide nothing,
 * as the GPU, which sees no mist in its depths, would ask for it all.
 * A cell is hidden if all it may draw (`shown`) is behind where the rays
 * round it stop.
 * Between rays, or from where the eye has moved since (DRIFT), it may
 * show: so each cell hidden asks the GPU whether it does after all.
 */
struct Hiding {
    occlusion: Occlusion,
    /** Where the eye was, its rays cast from, in a face's lattice coordinates. */
    centre: [f64; 3],
    far: f64,
    lod: Lod,
    lattice: Lattice,
    /** Whether cells are drawn flat (Request's `flat`). */
    flat: bool,
    /** Each grid level's heights, past which rays meet nothing; the tiles rays went through last, by level. */
    heights: Heights,
    cast: Vec<Arc<Tiles>>,
    /** Whether some rays met tiles not made, and may yet stop nearer. */
    open: bool,
}

impl Hiding {
    /** From the eye at `eye`. */
    fn new(eye: [f64; 3], far: f64, lod: Lod, lattice: Lattice, flat: bool, heights: Heights) -> Hiding {
        let occlusion = Occlusion::new([eye[0] * LAYER, eye[1], eye[2] * LAYER]);
        Hiding { occlusion, centre: eye, far, lod, lattice, flat, heights, cast: Vec::new(), open: true }
    }

    /**
     * Where what is drawn of `cell` may be: its box, a cell deeper, its
     * pillars running on, and half a cell nearer (without, the GPU missed
     * some that showed); drawn flat, its picture's plane and its edges
     * enlarged as far as its pillars reach from it (compositor.rs's Flat),
     * the cell's depth (2N layers) at most, a little more, across and along.
     */
    fn shown(&self, cell: &CellChunk) -> Box3 {
        let n = f64::from(1 << cell.level);
        let reach = if self.flat { 1.5 * n } else { 0.0 };
        let mut shown = centred(cell, [n * LAYER + reach, n + reach, n * LAYER]);
        shown[2] = [shown[2][0] - n * LAYER / 2.0 - reach, shown[2][1] + 2.0 * n * LAYER];
        shown
    }

    /** Whether nothing of `cell` shows: all it may draw (`shown`) is behind where the rays stop. */
    fn hides(&self, cell: &CellChunk) -> bool {
        self.occlusion.hides(self.shown(cell))
    }

    /** Whether to cast rays: never cast, or some met tiles not made, and tiles came since. */
    fn stale(&self, levels: &[Arc<Level>]) -> bool {
        self.open && (self.cast.len() != levels.len() || self.cast.iter().zip(levels).any(|(c, l)| !Arc::ptr_eq(c, &l.grids)))
    }

    /**
     * Casts on its rays that may yet stop nearer (occlusion.rs), through the
     * tiles of `levels`: half a voxel at a time, stopping where the nearest
     * site's voxel is whole; across a brick holding none, half a brick at a
     * time. Never, past the render distance, or out of the world's heights
     * going away. Where a tile is not made, it waits to go on from there.
     */
    fn cast(&mut self, levels: &[Arc<Level>]) {
        self.cast = levels.iter().map(|l| l.grids.clone()).collect();
        let eye = [self.centre[0] * LAYER, self.centre[1], self.centre[2] * LAYER];
        let Lattice { r, u, f } = self.lattice;
        let (lod, far, heights, layer) = (self.lod, self.far, self.heights, self.centre[2].floor() as i32);
        // Of a face's coordinates in units of grid 0, the world's: (a R + c F) / 2 + b U, a = x / LAYER, b = y, c = z / LAYER.
        let world = |p: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|d| (p[0] / LAYER * f64::from(r[d]) + p[2] / LAYER * f64::from(f[d])) / 2.0 + p[1] * f64::from(u[d]))
        };
        self.open = self.occlusion.cast(|[tx, ty], from| {
            let rising = world([tx, ty, 1.0])[1];
            let mut t = from.max(LAYER);
            // The tile read last, of its grid.
            let mut last: Option<(i32, TileId, &fcc::Grid)> = None;
            while t < far {
                let at = world([eye[0] + tx * t, eye[1] + ty * t, eye[2] + t]);
                let grid = lod.grid(lod.claiming((eye[2] + t) / LAYER, layer));
                let unit = f64::from(1 << grid);
                let (low, high) = heights[grid as usize];
                let q = at.map(|v| v / unit);
                if (q[1] >= high as f64 && rising >= 0.0) || (q[1] < low as f64 && rising <= 0.0) {
                    break;
                }
                let site = nearest_site(q);
                let id = site.map(|v| (v as i64).div_euclid(TILE) as i32);
                if last.is_none_or(|(g, i, _)| g != grid || i != id) {
                    last = levels[grid as usize].tiles.get(&id).map(|t| (grid, id, &*t.grid));
                }
                let mut step = f64::from(fcc::BRICK) / 2.0 * unit;
                match last {
                    None => return (f64::INFINITY, Some(t)),
                    Some((.., g)) if g.brick(site[0], site[1], site[2]) >= 0 => {
                        if g.get(site[0], site[1], site[2]).1 == fcc::ALPHA {
                            return (eye[2] + t, None);
                        }
                        step = unit / 2.0;
                    }
                    Some(_) => (),
                }
                t += step;
            }
            (f64::INFINITY, None)
        });
    }
}

/** The site of the FCC lattice (coordinates adding up to an even number) nearest point `q`. */
fn nearest_site(q: [f64; 3]) -> [i32; 3] {
    let mut s = q.map(f64::round);
    if (s[0] + s[1] + s[2]) as i64 & 1 != 0 {
        let d = (0..3).max_by(|&a, &b| (q[a] - s[a]).abs().total_cmp(&(q[b] - s[b]).abs())).expect("three");
        s[d] += if q[d] > s[d] { 1.0 } else { -1.0 };
    }
    s.map(|v| v as i32)
}

/** The box of half widths `half` about a cell's site, in a face's coordinates in units of grid 0 (occlusion's Box3). */
fn centred(cell: &CellChunk, half: [f64; 3]) -> Box3 {
    let s = cell.site.map(|v| f64::from(v << cell.atoms));
    let at = [s[0] * LAYER, s[1], s[2] * LAYER];
    std::array::from_fn(|d| [at[d] - half[d], at[d] + half[d]])
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
     * By face, its cells picked last, of whole voxels and of medium (each
     * by its Lod, Lod::for_medium); and by face and cell, the chunks of
     * those that came and went, or that a bake found empty, and the render
     * that last looked them up or let them go, kept a while: what else was
     * known of a cell is found again at less cost than kept.
     */
    picks: Vec<[Option<Pick>; 2]>,
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
    /** Whether its picks hide cells (Look's `hides`): when that changes, they are picked again. */
    hides: bool,
    /** What the compositor finds of the probes; by their ids, each's face and cell; how many it found showing. */
    feedback: Arc<Feedback>,
    probes: FastMap<u32, (usize, CellChunk)>,
    found: usize,
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

/**
 * Whether a cell may hold voxels that show: whether any of its level's
 * meet it (Level::meets), and not all are buried, every voxel's neighbours
 * whole (Level::buried), so that no ray from anywhere enters one; of its
 * medium, whether any see-through ones meet the least cell about its site
 * that holds all its medium reads (medium_reads), its neighbours' slabs
 * too, beside which it draws its own. Once it has its tiles.
 */
fn may_hold(cell: &CellChunk, lattice: Lattice, level: &Level) -> bool {
    let centre = lattice.world(cell.site);
    if !cell.medium {
        // Its voxels' sites are within its spacing of its centre along each axis, their neighbours one more.
        let r = cell.spacing() + 1;
        return level.meets(centre, cell.spacing(), false) && !level.buried(centre.map(|v| v - r), centre.map(|v| v + r));
    }
    // A cell of radius r holds |x| + |y| ≤ r of each pair of axes; of a box, its corners, a unit more for world()'s halves.
    let (min, max) = medium_reads(cell);
    let corners = (0..8).map(|k| lattice.world(std::array::from_fn(|d| if k >> d & 1 == 0 { min[d] } else { max[d] })));
    let radius = corners.flat_map(|w| (0..3).map(move |d| (w[d] - centre[d]).abs() + (w[(d + 1) % 3] - centre[(d + 1) % 3]).abs()));
    level.meets(centre, radius.max().expect("corners") + 1, true)
}

/**
 * Casts `pick`'s hiding's rays, if it hides and they are to be cast; then,
 * if they were or there are `fresh` cells to bake, those it hides of the
 * cells that wait for tiles or bakes are hidden, neither baked nor their
 * tiles asked for, each with a probe. Whether any were.
 */
fn hide(pick: &mut Pick, fresh: bool, levels: &[Arc<Level>], asking: &mut Asking) -> bool {
    let Some(hiding) = &mut pick.hiding else { return false };
    let cast = hiding.stale(levels);
    if cast {
        hiding.cast(levels);
    }
    if !cast && !fresh {
        return false;
    }
    let showing = &pick.showing;
    let hides = |cell: &CellChunk| !showing.contains(cell) && hiding.hides(cell);
    let mut hidden = Vec::new();
    pick.waiting.retain(|&(cell, tiles, wanted)| !hides(&cell) || (hidden.push((cell, tiles, wanted, None)), false).1);
    pick.baking.retain(|&cell, &mut (r, wanted)| !hides(&cell) || (hidden.push((cell, r.tiles, wanted, Some(r))), false).1);
    let any = !hidden.is_empty();
    for (cell, tiles, wanted, request) in hidden {
        if let Some(seen) = pick.picked.at(&cell) {
            *seen = Seen::Hidden;
        }
        let probe = asking.probe(cell, hiding.shown(&cell));
        pick.hidden.push(Hidden { cell, tiles, wanted, request, probe });
    }
    any
}

/** Takes cell `h` of face `f`'s `pick` out of hiding: it waits for tiles or its bake again, or, baked meanwhile, is drawn. */
fn unhide(f: usize, pick: &mut Pick, h: Hidden) {
    let Some(seen) = pick.picked.at(&h.cell) else { return };
    match (&*seen, h.request) {
        (Seen::Hidden, Some(r)) => {
            *seen = Seen::Bake(h.tiles);
            pick.baking.insert(h.cell, (r, h.wanted));
        }
        (Seen::Hidden, None) => {
            *seen = Seen::Tiles(h.tiles);
            pick.waiting.push((h.cell, h.tiles, h.wanted));
        }
        (Seen::Baked(chunk), _) => pick.chunks.push(Placement { face: f, cell: h.cell, chunk: chunk.clone() }),
        _ => (),
    }
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
        let baked = Arc::new(baked);
        let wake = baked.clone();
        let feedback = Arc::new(Feedback { found: Mutex::default(), wake: Box::new(move || wake()) });
        let bakery = Bakery::new(
            world.threads,
            move || baked(),
            move || {
                let mut baker = Baker::new(&palette);
                if let Some(light) = &light {
                    let mut light = light();
                    baker.light = Some(Box::new(move |level, site| light.faces(level as u32, site.map(i64::from))));
                }
                baker
            },
        );
        Renderer {
            picks: faces.iter().map(|_| [None, None]).collect(),
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
            hides: true,
            feedback: feedback.clone(),
            probes: FastMap::default(),
            found: 0,
            tiles: (0..GRID_LEVELS).map(|_| Arc::new(Tiles::new(0, []))).collect(),
            frame: Frame {
                chunks: Arc::default(),
                number: number(),
                changes: Arc::new([]),
                farthest: 0.0,
                relief,
                flat,
                feedback,
                heard: 0,
            },
            stats: Stats { baking: 0, queued: 0, tiles: Vec::new(), hidden: 0, found: 0 },
            ids: FastSet::default(),
            changes: VecDeque::new(),
        }
    }

    /** The chunks the picks of `look` compose of the world's tiles, each level's in `levels`, and what is missing for the rest. */
    pub fn render(&mut self, look: &Look, levels: &[Arc<Level>]) -> (Frame, &Stats) {
        let mut changed = false;
        if look.light.is_some() != self.lit {
            self.cells.clear();
            self.picks.iter_mut().for_each(|p| *p = [None, None]);
            self.lit = look.light.is_some();
            changed = true;
        }
        if look.hides != self.hides {
            self.picks.iter_mut().for_each(|p| *p = [None, None]);
            self.probes.clear();
            self.hides = look.hides;
            changed = true;
        }
        // Cells whose probes the GPU found showing are unhidden, and not hidden again while picked.
        let (found, heard) = self.feedback.take();
        let mut asked = vec![false; self.faces.len()];
        for id in found {
            let Some((f, _)) = self.probes.remove(&id) else { continue };
            let Some(pick) = self.picks[f][0].as_mut() else { continue };
            let Some(k) = pick.hidden.iter().position(|h| h.probe.id == id) else { continue };
            let h = pick.hidden.swap_remove(k);
            pick.showing.insert(h.cell);
            unhide(f, pick, h);
            (asked[f], changed, self.found) = (true, true, self.found + 1);
        }
        self.frame.heard = heard;
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
        let whole = Lod::new(look.shift, look.detail);
        for (f, asked) in asked.into_iter().enumerate() {
            let tangents = view.face_tangents(&self.faces[f]);
            // Back in view, its cells look for every tile that came meanwhile.
            let back = tangents.is_some() && !self.shown[f];
            changed |= tangents.is_some() != self.shown[f];
            self.shown[f] = tangents.is_some();
            let Some(tangents) = tangents else { continue };
            let local = to_local(&self.faces[f].axes, camera);
            let eye = [local[0] / LAYER, local[1], local[2] / LAYER];
            for lod in [whole, whole.for_medium()] {
                let part = usize::from(lod.medium);
                if self.picks[f][part].as_ref().is_some_and(|p| p.picked.covers(&lod, look.far, eye, tangents)) {
                    // Unhidden, what waits for tiles looks for those that came meanwhile, as back in view.
                    let all = back || asked;
                    changed |= self.look_up(f, part, if all { &[true; GRID_LEVELS] } else { &arrived }, levels, view.focal, local);
                    continue;
                }
                let [t0, t1, t2, t3] = tangents;
                let eyes = eye.map(|v| [v - DRIFT, v + DRIFT]);
                // Within the face's rays: |x| + √2 |y| ≤ z.
                let y = std::f64::consts::FRAC_1_SQRT_2;
                let wide = [(t0 - TURN).max(-1.0), (t1 + TURN).min(1.0), (t2 - TURN).max(-y), (t3 + TURN).min(y)];
                let mut old = self.picks[f][part].take().map(|p| {
                    p.hidden.iter().for_each(|h| _ = self.probes.remove(&h.probe.id));
                    p.picked
                });
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
            // Hidden from where the eye was at the last pick, maybe not from where it is.
            if matches!(seen, Seen::Hidden) {
                *seen = Seen::Unknown;
            }
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
                *seen = if may_hold(&cell, lattice, source) { Seen::Bake(t) } else { Seen::Empty };
            }
            let wanted = Wanted { distance: distance(&cell, eye), in_view: lod.composes(&cell, eye, far, tangents) };
            match seen {
                Seen::Unknown | Seen::Hidden | Seen::Empty => (),
                &mut Seen::Tiles(t) => waiting.push((cell, t, wanted)),
                &mut Seen::Bake(t) => _ = baking.insert(cell, (request(cell, t), wanted)),
                Seen::Baked(chunk) => chunks.push(Placement { face: f, cell, chunk: chunk.clone() }),
            }
        });
        let hiding = (!lod.medium && self.hides).then(|| Hiding::new(eye, far, *lod, lattice, self.flat, self.heights));
        let mut pick = Pick { picked, chunks, waiting, baking, hiding, hidden: Vec::new(), showing: FastSet::default() };
        _ = hide(&mut pick, true, levels, &mut Asking { face: f, next_id: &mut self.next_id, probes: &mut self.probes });
        self.picks[f][usize::from(lod.medium)] = Some(pick);
    }

    /** Requests to bake a cell of face `f` that reads given tiles, the eye at `local` seeing at `focal`. */
    fn request(&self, f: usize, focal: f64, local: [f64; 3]) -> impl Fn(CellChunk, [TileId; 2]) -> Request + use<> {
        let (lit, relief, flat) = (self.lit, self.relief, self.flat);
        move |cell, tiles| {
            let block = super::compositor::block(depths(&cell, local)[1].max(1.0), focal);
            Request { face: f, cell, tiles, block, lit, relief, flat }
        }
    }

    /**
     * Of face `f`'s cells that wait for tiles, of whole voxels or medium
     * (`part`, 1), those whose tiles arrived (by level, `arrived`) are to
     * bake, or hold nothing. Whether any were.
     */
    fn look_up(
        &mut self,
        f: usize,
        part: usize,
        arrived: &[bool; GRID_LEVELS],
        levels: &[Arc<Level>],
        focal: f64,
        local: [f64; 3],
    ) -> bool {
        let Some(mut pick) = self.picks[f][part].take() else { return false };
        let (lattice, request) = (self.faces[f].lattice, self.request(f, focal, local));
        let before = pick.waiting.len();
        pick.waiting.retain(|&(cell, t, wanted)| {
            let level = cell.atoms as usize;
            let source = &levels[level];
            if !arrived[level] || !source.grids.has_all(t) {
                return true;
            }
            let seen = if may_hold(&cell, lattice, source) {
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
        let hid = hide(&mut pick, changed, levels, &mut Asking { face: f, next_id: &mut self.next_id, probes: &mut self.probes });
        self.picks[f][part] = Some(pick);
        changed || hid
    }

    /** The frame of what the faces in view composed, and what they want: the chunks to bake, and the tiles, nearest first. */
    fn gather(&mut self, look: &Look) {
        let picks = || self.picks.iter().zip(&self.shown).filter(|p| *p.1).flat_map(|p| p.0.iter().flatten());
        let mut chunks: Vec<Placement> = picks().flat_map(|p| p.chunks.iter().cloned()).collect();
        // Their planes through their sites.
        let farthest = chunks
            .iter()
            .map(|p| {
                let [near, far] = depths(&p.cell, to_local(&self.faces[p.face].axes, look.camera));
                (near + far) / 2.0
            })
            .fold(look.far, f64::max);
        self.frame.farthest = farthest;
        // The probes, after all that draws.
        let shown =
            self.picks.iter().enumerate().zip(&self.shown).filter(|p| *p.1).flat_map(|((f, p), _)| p.iter().flatten().map(move |p| (f, p)));
        chunks.extend(shown.flat_map(|(face, p)| p.hidden.iter().map(move |h| Placement { face, cell: h.cell, chunk: h.probe.clone() })));
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
                within(t).filter(|&id| !source.has(id)).map(move |id| (level, id, wanted.distance))
            })
            .collect();
        tiles.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut listed = FastSet::default();
        self.stats = Stats {
            baking: baking.iter().filter(|b| b.1.in_view).count(),
            queued: baking.len(),
            tiles: tiles.into_iter().filter(|t| listed.insert((t.0, t.1))).map(|t| (t.0, t.1)).collect(),
            hidden: picks().map(|p| p.hidden.len()).sum(),
            found: self.found,
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
            if let Some(pick) = self.picks[r.face][usize::from(r.cell.medium)].as_mut()
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use worldgen::generator::materials_of;
    use worldgen::light::make_tile;

    use super::*;
    use crate::world::Tile;

    /**
     * The cells may_hold says hold nothing that shows bake empty, of every
     * face, in relief and as splats: a world's cells of 8·FCC of a grid
     * (level 3 over it) around its spawn, all whose reads its tiles there
     * hold; and some cells meeting voxels are buried.
     */
    #[test]
    fn cells_said_to_hold_nothing_bake_empty() {
        for (name, grid) in [("forest", 0), ("moor", 0), ("valley", 2)] {
            let world = (worlds::by_name(name).expect("world").make)(1);
            let spawn = world.spawn.map(|v| (v >> grid) as i32);
            let corner = spawn.map(|v| (v as f64 / TILE as f64).round() as i32);
            let heights = world.heights_at(grid);
            let tiles: HashMap<TileId, Arc<Tile>> = within([corner.map(|v| v - 1), corner])
                .map(|id| (id, Arc::new(Tile::new(id, Arc::new(make_tile(&world.generator, grid, heights, id.map(i64::from)))))))
                .collect();
            let level = Level::new(tiles);
            let mut baker = Baker::new(&materials_of(&world.generator).iter().map(|m| m.rgb()).collect::<Vec<_>>());
            baker.share_tiles(grid as usize, level.grids.clone());
            let (mut buried, mut empty) = (0, 0);
            for (face, &lattice) in fcc::lattices().iter().enumerate() {
                // The spawn in the face's coordinates: R·p, U·p, F·p.
                let dot = |v: [i32; 3]| (0..3).map(|d| v[d] * spawn[d]).sum::<i32>();
                let middle = [lattice.r, lattice.u, lattice.f].map(|v| dot(v).div_euclid(16) * 2);
                // Sites of 8·FCC, in 8's: of one parity.
                let sites =
                    within([middle.map(|v| v - 10), middle.map(|v| v + 10)]).filter(|p| (p[0] ^ p[2]) & 1 == 0 && (p[1] ^ p[2]) & 1 == 0);
                for at in sites {
                    let cell = CellChunk { level: grid as i32 + 3, atoms: grid as i32, site: at.map(|v| v * 8), medium: false };
                    let (_, min, max) = chunk_reads(&cell);
                    if !box_tiles(lattice, heights, min, max).is_some_and(|t| level.grids.has_all(t)) || may_hold(&cell, lattice, &level) {
                        continue;
                    }
                    empty += 1;
                    buried += usize::from(level.meets(lattice.world(cell.site), cell.spacing(), false));
                    for relief in [true, false] {
                        let baked = baker.bake_cell(face, &cell, relief);
                        assert!(baked.count == 0 && baker.columns.is_empty(), "{name}: face {face}'s {cell:?} shows (relief: {relief})");
                    }
                }
            }
            assert!(buried > 0 && buried < empty, "{name}: of {empty} cells holding nothing that shows, {buried} buried");
        }
    }
}
