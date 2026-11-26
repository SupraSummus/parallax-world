//! A generated world's tiles: each level's voxels are tiles fixed in the
//! world, cubes TILE sites of the level a side (worldgen's tile_box), no
//! axis apart from the others: asked for, made and dropped by how far they
//! are from the camera, as the crow flies. Of each, only what may show is
//! generated (worldgen's tiles.rs): none of it above or below the world's
//! heights, nor where its layers say it is empty or buried; on the
//! world's threads, which share the world and what it keeps, a tile with
//! those above and below it if the world has heights (made_with), and
//! indexed there. Lit, a tile's sites are made into the light (Shared::make_tiles),
//! which reads them, or are those it made: once, for both.
//! Which tiles, and in what order, is up to the engine drawing them
//! (Wants): every update hands over what it asks for, most wanted first,
//! replacing the previous list. No other tile is made but with one asked
//! for, so turning asks for what comes into view. Tiles farther from the
//! camera than the engine may ask for (Wants's reach) are dropped, and
//! the light's with them (worldgen's Shared). A tile
//! never changes, so a chunk baked once all the tiles it reads are in
//! stays right whatever comes after (lattice's Tiles). Given light (worldgen's Shared),
//! the bakers' Lights read its tiles, each site as it is, where
//! they read them. Asked for light, for the voxels engine
//! (voxels.rs), the bricks of tiles made that its rays asked for are lit,
//! their voxels' light found: only what shows,
//! which in a world that is not flat is little of all a ball around the
//! eye holds. Lit anew (relight), the tiles are kept, their light found again.

#[cfg(test)]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, OnceLock};

use lattice::GRID_LEVELS;
pub use lattice::TileId;
use lattice::{ALPHA, BRICK, BRICK_SITES, Blocks, FastSet, Grid, Tiles, alpha_of, medium};
use light::{Light, Shared};
use worldgen::tiles::{made_with, make_tiles};

use crate::engine::voxels;
use crate::threads::Post;
use crate::view::Vec3;

/** Tiles are worldgen's: 2^TILE_SHIFT sites of their level wide. */
pub const TILE: i64 = worldgen::tiles::TILE;
pub const TILE_SHIFT: u32 = TILE.trailing_zeros();
const _: () = assert!(TILE == 1 << TILE_SHIFT);
/**
 * Sites past an engine's reach that tiles are made within, and how much
 * farther they are dropped: apart, so that a tile dropped is not asked
 * for a step on. Chosen, not derived: the levels no ray reads have no
 * reach, and the light of what is lit reads their tiles, the coarsest's
 * as far off as the world is high (worldgen's `top_steps`); nearer, those were made every frame.
 */
const KEPT: f64 = 256.0;
const DROPPED: f64 = 128.0;

/** Each grid level's heights (World::heights_at). */
pub type Heights = [(i64, i64); GRID_LEVELS];

/** The heights of `world` for each grid level. */
fn heights(world: &worldgen::World) -> Heights {
    std::array::from_fn(|level| world.heights_at(level as u32))
}

/**
 * A generated tile: its voxels, which of its bricks are whole and which
 * hold a medium, and, lit, its bricks' light as the voxels engine takes
 * it, found as it asks for them.
 */
pub struct Tile {
    pub grid: Arc<Grid>,
    /** By place in the grid's bricks, a bit each: whether all its sites are whole voxels; whether any is a medium's. */
    whole: Arc<[u64]>,
    media: Arc<[u64]>,
    /** By where a brick's sites are in the grid's cells (in bricks), voxels::light of it, once found. */
    lights: Box<[OnceLock<Box<[u32]>>]>,
}

/**
 * A level's tiles made so far, and their voxels as a walking body reads
 * them: as they were when it was taken. Both are held in blocks (lattice's Blocks),
 * so that the next, some tiles come or gone, costs as those do.
 */
#[derive(Clone)]
pub struct Level {
    pub tiles: Blocks<Arc<Tile>>,
    pub grids: Arc<Tiles>,
}

impl Tile {
    /** Tile `id`, unlit. */
    pub fn new(grid: Arc<Grid>) -> Tile {
        let whole = bricks(&grid, |sites| sites.iter().all(|&look| alpha_of(look) == ALPHA));
        let media = bricks(&grid, |sites| sites.iter().any(|&look| medium(look)));
        Tile { lights: unlit(&grid), grid, whole, media }
    }

    /** Tile `self` with none of its light found. */
    pub fn unlit(&self) -> Tile {
        Tile { lights: unlit(&self.grid), grid: self.grid.clone(), whole: self.whole.clone(), media: self.media.clone() }
    }

    /** The light of the brick whose sites start at `first` in the grid's cells (in bricks), if found. */
    pub fn light(&self, first: usize) -> Option<&[u32]> {
        self.lights[first].get().map(|l| &l[..])
    }

    /** Finds the light of its brick whose sites start at `corner`, of `level`, by `light`, if it has voxels and not found yet. */
    pub fn lit(&self, level: u32, corner: [i32; 3], light: &mut Light) {
        let Ok(first) = usize::try_from(self.grid.brick(corner[0], corner[1], corner[2])) else { return };
        self.lights[first].get_or_init(|| voxels::light(&self.grid, level, corner, light));
    }
}

/** No light found for any brick of `grid`. */
fn unlit(grid: &Grid) -> Box<[OnceLock<Box<[u32]>>]> {
    (0..grid.cells.len() / BRICK_SITES).map(|_| OnceLock::new()).collect()
}

/**
 * By place in `grid`'s bricks, a bit each: whether `holds` the looks of
 * its sites. A brick with no voxels holds none; slots past the grid's box
 * neither, so a brick it cuts is not whole.
 */
fn bricks(grid: &Grid, holds: impl Fn(&[u32]) -> bool) -> Arc<[u64]> {
    let mut bits = vec![0; grid.bricks.len().div_ceil(64)];
    for (place, &at) in grid.bricks.iter().enumerate() {
        if let Ok(at) = usize::try_from(at)
            && holds(&grid.cells[at * BRICK_SITES..(at + 1) * BRICK_SITES])
        {
            bits[place / 64] |= 1 << (place % 64);
        }
    }
    bits.into()
}

impl Level {
    pub fn new(tiles: impl IntoIterator<Item = (TileId, Arc<Tile>)>) -> Level {
        let tiles: Blocks<Arc<Tile>> = tiles.into_iter().collect();
        let grids = Arc::new(Tiles::new(TILE_SHIFT, tiles.iter().map(|(id, t)| (id, t.grid.clone()))));
        Level { tiles, grids }
    }

    /** Takes `tile` as its tile `id`: its voxels are others' than those read so far (`grids`). */
    fn insert(&mut self, id: TileId, tile: Arc<Tile>) {
        Arc::make_mut(&mut self.grids).insert(id, tile.grid.clone());
        self.tiles.insert(id, tile);
    }

    /** Lets its tile `id` go, if it has it. */
    fn remove(&mut self, id: TileId) -> Option<Arc<Tile>> {
        let tile = self.tiles.remove(id)?;
        Arc::make_mut(&mut self.grids).remove(id);
        Some(tile)
    }

    /**
     * Whether a voxel of its tiles, or (`see_through`) one of a medium, is
     * in the box of sites from `lo` to `hi`, inclusive: read site by site
     * if it is small, else brick by brick, so it may say yes wrongly of a
     * large one, never no.
     */
    pub fn meets(&self, lo: [i32; 3], hi: [i32; 3], see_through: bool) -> bool {
        if (0..3).map(|d| i64::from(hi[d] - lo[d] + 1)).product::<i64>() <= BRICK_SITES as i64 {
            let kept = |look| if see_through { medium(look) } else { look != 0 };
            return within([lo, hi]).any(|q| self.grids.at(q[0], q[1], q[2]).is_some_and(kept));
        }
        self.bricks(
            lo,
            hi,
            |tile, place| if see_through { tile.media[place / 64] >> (place % 64) & 1 != 0 } else { tile.grid.bricks[place] >= 0 },
        )
        .any(|b| b)
    }

    /**
     * Whether every site from `lo` to `hi`, inclusive, holds a whole voxel:
     * whether all the bricks holding them are whole, all their tiles made.
     * So it may say no wrongly, never yes.
     */
    pub fn buried(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        let tile = |v: i32| (v as i64).div_euclid(TILE) as i32;
        within([lo.map(tile), hi.map(tile)]).all(|id| {
            let Some(t) = self.tiles.get(id) else { return false };
            let g = &t.grid;
            let tile_lo = id.map(|v| (v as i64 * TILE) as i32);
            (0..3).all(|d| lo[d].max(tile_lo[d]) >= g.min[d] && hi[d].min(tile_lo[d] + TILE as i32 - 1) < g.min[d] + g.n[d])
        }) && self.bricks(lo, hi, |tile, place| tile.whole[place / 64] >> (place % 64) & 1 != 0).all(|b| b)
    }

    /** What `of` says of each brick of its tiles' grids that the box from `lo` to `hi` meets, given its tile and its place among the grid's bricks. */
    fn bricks<'a>(&'a self, lo: [i32; 3], hi: [i32; 3], of: impl Fn(&Tile, usize) -> bool + 'a) -> impl Iterator<Item = bool> + 'a {
        let tile = |v: i32| (v as i64).div_euclid(TILE) as i32;
        within([lo.map(tile), hi.map(tile)]).flat_map(move |id| {
            let found = self.tiles.get(id).map(|t| {
                let g = &t.grid;
                let count = g.n.map(|v| (v + BRICK - 1) / BRICK);
                let tile_lo = id.map(|v| (v as i64 * TILE) as i32);
                // The box within the grid, in its bricks.
                let from: [i32; 3] = std::array::from_fn(|d| (lo[d].max(tile_lo[d]).max(g.min[d]) - g.min[d]) / BRICK);
                let to: [i32; 3] = std::array::from_fn(|d| {
                    (hi[d].min(tile_lo[d] + TILE as i32 - 1).min(g.min[d] + g.n[d] - 1) - g.min[d]).div_euclid(BRICK)
                });
                within([from, to]).map(|[i, j, k]| of(t, ((k * count[1] + j) * count[0] + i) as usize)).collect::<Vec<_>>()
            });
            found.unwrap_or_default()
        })
    }
}

/** The points from `lo` to `hi`, inclusive: tiles, or cells. */
pub fn within([lo, hi]: [[i32; 3]; 2]) -> impl Iterator<Item = [i32; 3]> {
    (lo[2]..=hi[2]).flat_map(move |k| (lo[1]..=hi[1]).flat_map(move |j| (lo[0]..=hi[0]).map(move |i| [i, j, k])))
}

/** How the tiles are lit: not, by what reads them (the bakers' Lights), or here, the bricks the engine asks for (the voxels'). */
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lit {
    Not,
    Reading,
    Bricks,
}

/**
 * What an engine reads of the world: each level's tiles as far around the
 * camera as it may ask for them whichever way it looks, `reach`, in the
 * level's sites (none if it reads none; those farther are dropped), lit
 * as `lit` says; and what it asks for, most wanted first: the tiles it
 * lacks for what is in view, and, lit here, the bricks of tiles made to light.
 */
pub struct Wants {
    pub reach: [Option<f64>; GRID_LEVELS],
    pub lit: Lit,
    pub asked: Arc<Vec<(usize, Piece)>>,
}

/** What a worker does, of a level: makes a tile, or lights a brick of one made, where its sites start. */
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Piece {
    Tile(TileId),
    Brick([i32; 3]),
}

/** A piece of a level, and the tile a brick is of. */
type Job = (usize, Piece, Option<Arc<Tile>>);

/** What the thread that draws has the workers do, in turn. */
enum Order {
    /**
     * Make or light `wanted`, most wanted first, in place of what was
     * wanted, but what is under way; into the light if `lit` (Wants's);
     * free what is `gone`; and forget the light of `level` far from
     * `camera`, past its `keep`.
     */
    Want { wanted: VecDeque<Job>, lit: bool, gone: Vec<Box<dyn Send>>, level: usize, camera: Vec3, keep: f64 },
    /** Light by this, from now on (World::relight). */
    Relight(Shared),
}

/** What the workers tell the thread that draws: pieces taken to make or light, and a piece done. */
enum Done {
    Took(Vec<(usize, Piece)>),
    Made(Made),
}

/**
 * The workers' alone: the orders, and as they say, the pieces to make or
 * light, most wanted first, those being made or lit, the light, how many
 * times it changed, whether tiles are made into it (`lit`), what to free,
 * and where the light is to forget from, if asked since it last did.
 */
struct Queue {
    orders: Receiver<Order>,
    wanted: VecDeque<Job>,
    in_flight: FastSet<(usize, Piece)>,
    shared: Option<Shared>,
    lighting: u32,
    lit: bool,
    gone: Vec<Box<dyn Send>>,
    forget: Option<(usize, Vec3, f64)>,
}

impl Queue {
    /** Takes the orders given since it last did; false once the world is gone. */
    fn read(&mut self) -> bool {
        loop {
            let order = match self.orders.try_recv() {
                Ok(order) => order,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            };
            match order {
                Order::Want { mut wanted, lit, mut gone, level, camera, keep } => {
                    wanted.retain(|job| !self.in_flight.contains(&(job.0, job.1)));
                    self.gone.append(&mut gone);
                    (self.wanted, self.lit, self.forget) = (wanted, lit, Some((level, camera, keep)));
                }
                Order::Relight(shared) => {
                    self.shared = Some(shared);
                    self.lighting += 1;
                }
            }
        }
    }
}

/** A piece done: a tile made, or a brick lit (its light in its tile). */
type Made = (usize, Piece, Option<Arc<Tile>>);

/**
 * The world's tiles as the thread that draws has them: it orders its
 * threads to make and light them, and follows what they do from what
 * they tell it (Done), waiting for none of them (threads.rs).
 */
pub struct World {
    /** Per level, the tiles made so far, as engines read them. */
    levels: Vec<Arc<Level>>,
    /** The level whose far tiles the next update drops: one each, in turn. */
    sweeping: usize,
    pub heights: Heights,
    /** The threads it makes tiles on, which engines may take as many of. */
    pub threads: usize,
    /** The world its threads make, and the light (if any) it is made into, for Lights of the bakers' own. */
    world: Arc<worldgen::World>,
    shared: Option<Shared>,
    orders: Post<Order>,
    done: Receiver<Done>,
    /** Received by wait(), not taken yet. */
    early: Vec<Done>,
    /** What it last wanted, but what was taken since; and what was taken, not done yet: as its threads told. */
    wanted: FastSet<(usize, Piece)>,
    in_flight: FastSet<(usize, Piece)>,
    /** Each worker's clock (own_clock), once it runs. */
    #[cfg(target_os = "linux")]
    clocks: Arc<[OnceLock<libc::clockid_t>]>,
}

/** How far tile `id` of `level` is from the camera (lattice units), in the level's sites. */
fn distance(level: usize, camera: Vec3, id: TileId) -> f64 {
    let n = (1u64 << level) as f64;
    let off = |d: usize| {
        let (lo, hi) = ((id[d] as i64 * TILE) as f64, ((id[d] as i64 + 1) * TILE) as f64);
        (lo - camera[d] / n).max(camera[d] / n - hi).max(0.0)
    };
    (0..3).map(|d| off(d).powi(2)).sum::<f64>().sqrt()
}

/** The tiles of `level` within `reach` of the camera (lattice units) in the level's sites, nearest first. */
fn around(level: usize, camera: Vec3, reach: f64) -> Vec<(f64, TileId)> {
    // The first tile whose end is not short of `reach` below each way, and the last whose start is not past it above.
    let at = camera.map(|v| v / (1u64 << level) as f64 / TILE as f64);
    let r = reach / TILE as f64;
    let (lo, hi) = (at.map(|v| (v - r).ceil() as i32 - 1), at.map(|v| (v + r).floor() as i32));
    let mut around: Vec<(f64, TileId)> =
        within([lo, hi]).map(|id| (distance(level, camera, id), id)).filter(|(d, _)| *d <= reach).collect();
    around.sort_by(|a, b| a.0.total_cmp(&b.0));
    around
}

/**
 * Puts the calling thread, a worker, behind the thread that draws, which
 * waits for none: as many workers as cores would else keep it from its own.
 */
pub fn behind_drawing() {
    // SAFETY: setpriority only reads its arguments; on Linux, who 0 is this thread alone.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
}

/** The clock of the calling thread's time on a core, to read from another (World::worked). */
#[cfg(target_os = "linux")]
fn own_clock() -> libc::clockid_t {
    let mut clock = 0;
    // SAFETY: pthread_getcpuclockid only writes the clock of this thread, which is running.
    unsafe { libc::pthread_getcpuclockid(libc::pthread_self(), &mut clock) };
    clock
}

/** What makes tiles and lights bricks, on a thread of its own: the world all share, and a Light of its own. */
struct Worker {
    world: Arc<worldgen::World>,
    /** Made once there are tiles to light, and again when the light changes. */
    light: Option<(u32, Light)>,
}

/** A job taken off the queue, and what the queue said then: its light, how many times that changed, and whether it is read. */
type Taken = (Job, Option<Shared>, u32, bool);

impl Worker {
    fn new(world: Arc<worldgen::World>) -> Worker {
        Worker { world, light: None }
    }

    /** The most wanted job, if any, now under way, with the tiles made with its own; and the pieces it took. */
    fn take(&self, q: &mut Queue) -> Option<(Taken, Vec<(usize, Piece)>)> {
        let job = q.wanted.pop_front()?;
        let mut took = vec![(job.0, job.1)];
        // Those wanted that are made with it wait no more.
        if let Piece::Tile(id) = job.1 {
            took.extend(made_with(&self.world, job.0 as u32, id.map(i64::from)).map(|key| (job.0, Piece::Tile(key.map(|v| v as i32)))));
            let with: FastSet<_> = took.iter().copied().collect();
            q.wanted.retain(|job| !with.contains(&(job.0, job.1)));
        }
        q.in_flight.extend(took.iter().copied());
        Some(((job, q.shared.clone(), q.lighting, q.lit), took))
    }

    fn make(&mut self, ((level, piece, of), shared, lighting, lit): Taken) -> Vec<Made> {
        match piece {
            Piece::Brick(corner) => {
                self.light.take_if(|(l, _)| *l != lighting);
                let world = &self.world;
                let (_, light) =
                    self.light.get_or_insert_with(|| (lighting, Light::new(world.clone(), shared.expect("light to light tiles with"))));
                // A tile lit anew since (relight) is let go, and this light with it.
                of.expect("the tile of a brick").lit(level as u32, corner, light);
                vec![(level, piece, None)]
            }
            Piece::Tile(id) => {
                let key = id.map(i64::from);
                // Read by light, its sites are the light's: made once.
                let grids = match shared.filter(|_| lit) {
                    Some(shared) => shared.make_tiles(&self.world, level as u32, key),
                    None => make_tiles(&self.world, level as u32, key),
                };
                let tile = |(key, grid): ([i64; 3], Grid)| {
                    let id = key.map(|v| v as i32);
                    (level, Piece::Tile(id), Some(Arc::new(Tile::new(Arc::new(grid)))))
                };
                grids.into_iter().map(tile).collect()
            }
        }
    }
}

/**
 * A worker's: the orders as they come, the light's forgetting, the most
 * wanted piece at a time; parked when there is none, till the world is gone.
 */
fn work(worker: &mut Worker, queue: &Mutex<Queue>, send: &Sender<Done>) {
    let mut done = Vec::new();
    loop {
        let (gone, forget, taken) = {
            let mut q = queue.lock().expect("queue");
            for piece in done.drain(..) {
                q.in_flight.remove(&piece);
            }
            if !q.read() {
                return;
            }
            (std::mem::take(&mut q.gone), q.forget.take().zip(q.shared.clone()), worker.take(&mut q))
        };
        // What the world let go, and the light forgets, is long to free: here, not where it is drawn.
        drop(gone);
        if let Some(((level, camera, keep), shared)) = forget {
            drop(shared.forget(|l, key| l == level && far(level, camera, keep, key.map(|k| k as i32))));
        }
        let Some((taken, took)) = taken else {
            std::thread::park();
            continue;
        };
        // Taken, then done: so the thread that draws knows of each piece under way till it is done.
        _ = send.send(Done::Took(took.clone()));
        for made in worker.make(taken) {
            _ = send.send(Done::Made(made));
        }
        done = took;
    }
}

impl World {
    /**
     * `world`, generated (into `shared` if given,
     * the light it can be lit by) and indexed on `threads` threads; not lit yet.
     */
    pub fn new(world: worldgen::World, shared: Option<Shared>, threads: usize) -> World {
        assert!(threads > 0, "a world made on no threads");
        let world = Arc::new(world);
        let heights = heights(&world);
        let (send, done) = channel();
        #[cfg(target_os = "linux")]
        let clocks: Arc<[OnceLock<libc::clockid_t>]> = (0..threads).map(|_| OnceLock::new()).collect();
        let orders = Post::spawn(|orders| {
            let queue = Queue {
                orders,
                wanted: VecDeque::new(),
                in_flight: FastSet::default(),
                shared: shared.clone(),
                lighting: 0,
                lit: false,
                gone: Vec::new(),
                forget: None,
            };
            let queue = Arc::new(Mutex::new(queue));
            (0..threads)
                .map(|i| {
                    let (world, queue, send) = (world.clone(), queue.clone(), send.clone());
                    #[cfg(target_os = "linux")]
                    let clocks = clocks.clone();
                    move || {
                        behind_drawing();
                        #[cfg(target_os = "linux")]
                        clocks[i].set(own_clock()).expect("a clock once");
                        #[cfg(not(target_os = "linux"))]
                        let _ = i;
                        work(&mut Worker::new(world), &queue, &send)
                    }
                })
                .collect()
        });
        World {
            levels: (0..GRID_LEVELS).map(|_| Arc::new(Level::new([]))).collect(),
            sweeping: 0,
            heights,
            threads,
            world,
            shared,
            orders,
            done,
            early: Vec::new(),
            wanted: FastSet::default(),
            in_flight: FastSet::default(),
            #[cfg(target_os = "linux")]
            clocks,
        }
    }

    /** The tiles of `level` made so far. */
    pub fn level(&self, level: usize) -> &Arc<Level> {
        &self.levels[level]
    }

    /** A Light of the world for each who asks, if it was given light: it lights what they read from the tiles' light. */
    pub fn light(&self) -> Option<impl Fn() -> Light + Send + Sync + 'static> {
        let world = self.world.clone();
        self.shared.clone().map(|shared| move || Light::new(world.clone(), shared.clone()))
    }

    /**
     * The tiles of `level` within `reach` of the camera, in its sites, not
     * made yet, nearest first: what one who reads all around there lacks,
     * as a body does of the ground it walks on.
     */
    pub fn lacking(&self, level: usize, reach: f64, camera: Vec3) -> Vec<TileId> {
        around(level, camera, reach).into_iter().map(|(_, id)| id).filter(|&id| self.levels[level].tiles.get(id).is_none()).collect()
    }

    /** The light it is lit by, if any. */
    pub fn shared(&self) -> Option<&Shared> {
        self.shared.as_ref()
    }

    /**
     * Lights the world anew by `shared`, its own light under another
     * brightness or steps (Shared::under, Shared::stepping): the tiles made
     * are kept, their light to find again, and what is being lit lights
     * tiles let go. Engines that read its light (Lit) are to be made again.
     */
    pub fn relight(&mut self, shared: Shared) {
        self.shared = Some(shared.clone());
        self.orders.send(Order::Relight(shared));
        for level in &mut self.levels {
            *level = Arc::new(Level::new(level.tiles.iter().map(|(id, tile)| (id, Arc::new(tile.unlit())))));
        }
    }

    /** Tiles being made and bricks lit, or waiting to be. */
    pub fn making(&self) -> usize {
        self.wanted.len() + self.in_flight.len()
    }

    /** Whether it lacks a tile `wants` asks for. */
    pub fn lacks(&self, wants: &Wants) -> bool {
        wants.asked.iter().any(|&(level, piece)| matches!(piece, Piece::Tile(id) if self.levels[level].tiles.get(id).is_none()))
    }

    /** What it owes the view, as its threads last told: the tiles to make and the bricks to light, waiting or under way. */
    pub fn owed(&self) -> [usize; 2] {
        let pieces = self.wanted.iter().chain(&self.in_flight).map(|job| job.1);
        pieces.fold([0, 0], |[tiles, bricks], piece| match piece {
            Piece::Tile(_) => [tiles + 1, bricks],
            Piece::Brick(_) => [tiles, bricks + 1],
        })
    }

    /**
     * Seconds each of its threads has had on a core, so far: what its work
     * costs, whatever else the cores were busy with (none, off Linux).
     */
    pub fn worked(&self) -> Vec<f64> {
        #[cfg(target_os = "linux")]
        {
            let time = |&clock: &libc::clockid_t| {
                let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                // SAFETY: clock_gettime only writes `t`; the clock is of a worker, which never ends.
                unsafe { libc::clock_gettime(clock, &mut t) };
                t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
            };
            self.clocks.iter().filter_map(OnceLock::get).map(time).collect()
        }
        #[cfg(not(target_os = "linux"))]
        Vec::new()
    }

    /**
     * Waits until its threads tell of a piece taken or done: for those
     * that want all of a view (main.rs's `complete`), never in a frame.
     */
    pub fn wait(&mut self) {
        self.early.extend(self.done.recv());
    }

    /**
     * Takes the tiles made, drops those far from the camera (lattice
     * units), a level's each time, in turn (none comes to be far in a few
     * frames), and has done what `wants` asks for, most wanted first: the
     * tiles it names made, and, lit here, the bricks lit, what shows
     * (rays ask for them), small, and in a world that is not flat, little
     * of the tiles made. No other tile is made but with one it names
     * (made_with). What it lets go, its threads free: that is long.
     * Returns whether any level's tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, wants: &Wants) -> bool {
        let Wants { reach, lit, asked } = wants;
        let lit = *lit;
        assert!(lit == Lit::Not || self.shared.is_some(), "a world lit with no light");
        // Tiles are made no farther from the camera than `keep` (in the level's sites), and dropped once farther still.
        let keep = |level: usize| reach[level].unwrap_or(0.0) + KEPT;
        let mut changed = false;
        let mut made = Vec::new();
        for done in std::mem::take(&mut self.early).into_iter().chain(self.done.try_iter()) {
            match done {
                Done::Took(took) => {
                    for piece in took {
                        self.wanted.remove(&piece);
                        self.in_flight.insert(piece);
                    }
                }
                Done::Made(piece) => {
                    self.in_flight.remove(&(piece.0, piece.1));
                    made.push(piece);
                }
            }
        }
        let far = |level: usize, id: TileId| far(level, camera, keep(level), id);
        let mut gone: Vec<Box<dyn Send>> = Vec::new();
        for (level, piece, tile) in made {
            let (Piece::Tile(id), Some(tile)) = (piece, tile) else { continue };
            // Made with one asked for, a tile may be far, or here already: that one is kept, with its light.
            if far(level, id) || self.levels[level].tiles.get(id).is_some() {
                gone.push(Box::new(tile));
            } else {
                Arc::make_mut(&mut self.levels[level]).insert(id, tile);
                changed = true;
            }
        }
        let level = self.sweeping;
        self.sweeping = (level + 1) % GRID_LEVELS;
        let dropped: Vec<TileId> = self.levels[level].tiles.iter().map(|(id, _)| id).filter(|&id| far(level, id)).collect();
        if !dropped.is_empty() {
            let tiles = Arc::make_mut(&mut self.levels[level]);
            gone.extend(dropped.into_iter().filter_map(|id| tiles.remove(id)).map(|tile| Box::new(tile) as Box<dyn Send>));
            changed = true;
        }

        let (mut list, mut listed) = (VecDeque::new(), FastSet::default());
        let unlit = |tile: &Tile, [x, y, z]: [i32; 3]| usize::try_from(tile.grid.brick(x, y, z)).is_ok_and(|f| tile.light(f).is_none());
        for &(level, piece) in asked.iter() {
            let tiles = &self.levels[level].tiles;
            let job = match piece {
                Piece::Tile(id) => (distance(level, camera, id) <= keep(level) && tiles.get(id).is_none()).then_some(None),
                Piece::Brick(corner) => {
                    let tile = tiles.get(corner.map(|v| v.div_euclid(TILE as i32)));
                    tile.filter(|t| lit == Lit::Bricks && unlit(t, corner)).map(|t| Some(t.clone()))
                }
            };
            if let Some(of) = job
                && !self.in_flight.contains(&(level, piece))
                && listed.insert((level, piece))
            {
                list.push_back((level, piece, of));
            }
        }
        // One taken and done since what was done was read above may be asked for and made again: the second is dropped.
        self.wanted = listed;
        self.orders.send(Order::Want { wanted: list, lit: lit != Lit::Not, gone, level, camera, keep: keep(level) });
        changed
    }
}

/** Whether tile `id` of `level` is far enough from the camera (lattice units) to drop, kept within `keep` (World::update). */
fn far(level: usize, camera: Vec3, keep: f64, id: TileId) -> bool {
    distance(level, camera, id) > keep + DROPPED
}

/** For tests: the tiles of `level` of `world`, unlit, that meet the cube reaching `reach` sites each way from `centre`. */
#[cfg(test)]
pub fn tiles_about(world: &worldgen::World, level: u32, centre: [i32; 3], reach: i32) -> HashMap<TileId, Arc<Tile>> {
    let tile = |v: i32| v.div_euclid(TILE as i32);
    let mut tiles = HashMap::new();
    for id in within([centre.map(|v| tile(v - reach)), centre.map(|v| tile(v + reach - 1))]) {
        if !tiles.contains_key(&id) {
            let made = make_tiles(world, level, id.map(i64::from)).into_iter().map(|(key, grid)| (key.map(|v| v as i32), grid));
            tiles.extend(made.map(|(id, grid)| (id, Arc::new(Tile::new(Arc::new(grid))))));
        }
    }
    tiles.retain(|id, _| (0..3).all(|d| (tile(centre[d] - reach)..=tile(centre[d] + reach - 1)).contains(&id[d])));
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    /** The coarsest level's tiles around the origin, by tile, their bricks' light, each brick of voxels asked for, as the voxels engine asks. */
    fn lit(world: &mut World) -> Vec<(TileId, Vec<Option<Vec<u32>>>)> {
        let coarsest = GRID_LEVELS - 1;
        let reach = std::array::from_fn(|l| (l == coarsest).then_some(TILE as f64));
        let mut wants = Wants { reach, lit: Lit::Bricks, asked: Arc::default() };
        loop {
            world.update([0.0; 3], &wants);
            let lacking = world.lacking(coarsest, TILE as f64, [0.0; 3]).into_iter().map(|id| (coarsest, Piece::Tile(id)));
            let tiles = world.level(coarsest).tiles.iter().map(|(_, t)| t);
            let bricks = tiles.flat_map(|t| corners(&t.grid)).map(|c| (coarsest, Piece::Brick(c)));
            let asked: Vec<_> = lacking.chain(bricks).collect();
            if asked != *wants.asked {
                wants.asked = Arc::new(asked);
            } else if world.making() == 0 {
                break;
            } else {
                world.wait();
            }
        }
        let light =
            |t: &Tile| -> Vec<_> { (0..t.grid.cells.len() / lattice::BRICK_SITES).map(|f| t.light(f).map(<[u32]>::to_vec)).collect() };
        let mut lights: Vec<_> = world.level(coarsest).tiles.iter().map(|(id, t)| (id, light(t))).collect();
        lights.sort();
        assert!(lights.iter().any(|(_, l)| l.iter().any(Option::is_some)), "some light found");
        lights
    }

    /** Where the sites of each brick of `grid` with voxels start. */
    fn corners(grid: &Grid) -> impl Iterator<Item = [i32; 3]> + '_ {
        let count = grid.n.map(|v| (v + lattice::BRICK - 1) / lattice::BRICK);
        (grid.bricks.iter().enumerate()).filter(|(_, first)| **first >= 0).map(move |(place, _)| {
            let b = place as i32;
            let at = [b % count[0], b / count[0] % count[1], b / count[0] / count[1]];
            std::array::from_fn(|d| grid.min[d] + at[d] * lattice::BRICK)
        })
    }

    #[test]
    fn tiles_are_asked_for_alike_whichever_way_they_lie() {
        // Around a corner of tiles, each tile's turns and mirrors (the cube's 48) are as far, so asked for alike.
        let around = around(2, [0.0; 3], 3.0 * TILE as f64);
        let at: HashMap<TileId, f64> = around.iter().map(|&(d, id)| (id, d)).collect();
        assert!(around.windows(2).all(|w| w[0].0 <= w[1].0), "nearest first");
        assert!(at.contains_key(&[0, 3, 0]) && at.contains_key(&[0, -4, 0]), "above and below as far as along x and z");
        for (&id, &d) in &at {
            for turn in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
                for mirror in 0..8 {
                    let other: TileId = std::array::from_fn(|a| if mirror >> a & 1 == 1 { -1 - id[turn[a]] } else { id[turn[a]] });
                    assert_eq!(at.get(&other), Some(&d), "{id:?} and {other:?}");
                }
            }
        }
    }

    #[test]
    fn a_world_lit_anew_is_as_one_made_so() {
        use light::Brightness;
        let make = || (worlds::by_name("forest").expect("forest").make)(1);
        let (day, dusk) = (Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }, Brightness { sky: 0.3, sun: 0.1, glow: 1.0 });
        let mut world = World::new(make(), Some(Shared::new(day)), 4);
        let before = lit(&mut world);
        world.relight(world.shared().expect("light").under(dusk).stepping(3));
        let after = lit(&mut world);
        assert!(after != before, "the light changed");
        assert!(after == lit(&mut World::new(make(), Some(Shared::new(dusk).stepping(3)), 4)));
    }

    #[test]
    fn a_world_on_one_thread_is_as_one_on_some() {
        use light::Brightness;
        let make = || (worlds::by_name("forest").expect("forest").make)(1);
        let day = Brightness { sky: 1.0, sun: 1.0, glow: 0.0 };
        assert!(lit(&mut World::new(make(), Some(Shared::new(day)), 1)) == lit(&mut World::new(make(), Some(Shared::new(day)), 4)));
    }
}
