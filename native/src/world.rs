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
//! stays right whatever comes after (fcc's Tiles). Given light (worldgen's Shared),
//! the bakers' Lights read its tiles, each site as it is, where
//! they read them. Asked for light, for the voxels engine
//! (voxels.rs), the bricks of tiles made that its rays asked for are lit,
//! their voxels' light found: only what shows,
//! which in a world that is not flat is little of all a ball around the
//! eye holds. Lit anew (relight), the tiles are kept, their light found again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use fcc::GRID_LEVELS;
pub use fcc::TileId;
use fcc::{Grid, Tiles};
use light::{Light, Shared};
use worldgen::tiles::{made_with, make_tiles};

use crate::engine::voxels;
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
 * A generated tile: its voxels, the cells of the lattice they meet
 * (voxels.rs), by brick and in all, its whole bricks, and, lit, its
 * bricks' light as the voxels engine takes it, found as it asks for them.
 */
pub struct Tile {
    pub grid: Arc<Grid>,
    /** voxels::meets. */
    pub meets: Vec<u32>,
    pub met: voxels::Met,
    /** By place in the grid's bricks, a bit each: whether all its sites are whole voxels (whole). */
    pub whole: Arc<[u64]>,
    /** By where a brick's sites are in the grid's cells (in bricks), voxels::light of it, once found. */
    lights: Box<[OnceLock<Box<[u32]>>]>,
}

/** A level's tiles made so far, and their voxels as the bakers read them. */
pub struct Level {
    pub tiles: HashMap<TileId, Arc<Tile>>,
    pub grids: Arc<Tiles>,
}

impl Tile {
    /** Tile `id`, unlit. */
    pub fn new(id: TileId, grid: Arc<Grid>) -> Tile {
        let meets = voxels::meets(&grid);
        let met = voxels::Met::new(id, &grid, &meets);
        Tile { whole: whole(&grid), lights: unlit(&grid), grid, meets, met }
    }

    /** Tile `self` with none of its light found. */
    pub fn unlit(&self) -> Tile {
        let (grid, meets, met, whole) = (self.grid.clone(), self.meets.clone(), self.met.clone(), self.whole.clone());
        Tile { lights: unlit(&grid), grid, meets, met, whole }
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
    (0..grid.cells.len() / fcc::BRICK_SITES).map(|_| OnceLock::new()).collect()
}

/**
 * By place in `grid`'s bricks, a bit each: whether every site of the
 * brick holds a whole voxel. Slots past the grid's box hold none, so a
 * brick it cuts is not whole.
 */
fn whole(grid: &Grid) -> Arc<[u64]> {
    let mut bits = vec![0; grid.bricks.len().div_ceil(64)];
    for (place, &at) in grid.bricks.iter().enumerate() {
        let sites = usize::try_from(at).map(|at| at * fcc::BRICK_SITES..(at + 1) * fcc::BRICK_SITES);
        if sites.is_ok_and(|mut sites| sites.all(|i| fcc::alpha_of(grid.cells[i]) == fcc::ALPHA)) {
            bits[place / 64] |= 1 << (place % 64);
        }
    }
    bits.into()
}

impl Level {
    pub fn new(tiles: HashMap<TileId, Arc<Tile>>) -> Level {
        let grids = Arc::new(Tiles::new(TILE_SHIFT, tiles.iter().map(|(&id, t)| (id, t.grid.clone()))));
        Level { tiles, grids }
    }

    /**
     * Whether a voxel of its tiles, or (`see_through`) a see-through one,
     * meets the cell of `radius`·FCC about `centre`, a rhombic dodecahedron
     * as a voxel is of radius 1.
     * A cell smaller than those of 8·FCC is read site by site;
     * a larger one asks the cells of 8·FCC or 32·FCC that meet it (voxels::Met),
     * so of those sizes it is exact, and of the others it may say yes wrongly.
     */
    pub fn meets(&self, centre: [i32; 3], radius: i32, see_through: bool) -> bool {
        if radius < voxels::cell_size(0) {
            let kept = |look| if see_through { !matches!(fcc::alpha_of(look), 0 | fcc::ALPHA) } else { look != 0 };
            return meeting(centre, radius, 1).any(|q| self.grids.at(q[0], q[1], q[2]).is_some_and(kept));
        }
        let j = usize::from(radius >= voxels::cell_size(1));
        meeting(centre, radius, voxels::cell_size(j))
            .any(|k| within(voxels::Met::tiles(j, k)).any(|t| self.tiles.get(&t).is_some_and(|tile| tile.met.has(t, j, k, see_through))))
    }

    /**
     * Whether every site from `lo` to `hi`, inclusive, holds a whole voxel:
     * whether all the bricks holding them are whole (Tile's `whole`), all
     * their tiles made. So it may say no wrongly, never yes.
     */
    pub fn buried(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        let tile = |v: i32| (v as i64).div_euclid(TILE) as i32;
        within([lo.map(tile), hi.map(tile)]).all(|id| {
            let Some(t) = self.tiles.get(&id) else { return false };
            let g = &t.grid;
            // The box within the tile, by brick of its grid, whose box is to hold it.
            let (from, to): ([i32; 3], [i32; 3]) = (
                std::array::from_fn(|d| lo[d].max((id[d] as i64 * TILE) as i32) - g.min[d]),
                std::array::from_fn(|d| hi[d].min(((id[d] as i64 + 1) * TILE) as i32 - 1) - g.min[d]),
            );
            let bricks = g.n.map(|v| (v + fcc::BRICK - 1) / fcc::BRICK);
            (0..3).all(|d| from[d] >= 0 && to[d] < g.n[d])
                && within([from.map(|v| v / fcc::BRICK), to.map(|v| v / fcc::BRICK)]).all(|[i, j, k]| {
                    let place = ((k * bricks[1] + j) * bricks[0] + i) as usize;
                    t.whole[place / 64] >> (place % 64) & 1 != 0
                })
        })
    }
}

/**
 * The cells of `s`·FCC (about k · s, k's coordinates adding up to an even
 * number) that meet the cell of `radius`·FCC about `centre`: rhombic
 * dodecahedra meet where their centres are nearer than the sum of their
 * radii across each pair of faces. Of 1·FCC, the sites in it.
 */
fn meeting(centre: [i32; 3], radius: i32, s: i32) -> impl Iterator<Item = [i32; 3]> {
    let r = s + radius;
    let apart = move |k: [i32; 3], d: usize| (k[d] * s - centre[d]).abs();
    let near = [centre.map(|v| (v - r).div_euclid(s) + 1), centre.map(|v| (v + r - 1).div_euclid(s))];
    within(near).filter(move |k| (k[0] + k[1] + k[2]) & 1 == 0 && (0..3).all(|d| apart(*k, d) + apart(*k, (d + 1) % 3) < r))
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

/**
 * The pieces to make or light, most wanted first, those being made or
 * lit, the light they are lit by, how many times it changed, and whether
 * what asks reads it (Wants's `lit`): tiles are then made into it.
 */
#[derive(Default)]
struct Queue {
    wanted: VecDeque<Job>,
    in_flight: HashSet<(usize, Piece)>,
    shared: Option<Shared>,
    lighting: u32,
    lit: bool,
    /** What the world let go, for a worker to free: long, for the thread that draws. */
    gone: Vec<Box<dyn Send>>,
    /** Each worker's clock (own_clock). */
    #[cfg(target_os = "linux")]
    clocks: Vec<libc::clockid_t>,
}

impl Queue {
    /** Takes the tiles `with` tile `id` of `level` (made_with), to be made with it: those wanted wait no more. */
    fn take(&mut self, level: usize, with: impl Iterator<Item = TileId>) {
        let with: HashSet<Piece> = with.map(Piece::Tile).collect();
        self.wanted.retain(|job| job.0 != level || !with.contains(&job.1));
        self.in_flight.extend(with.into_iter().map(|piece| (level, piece)));
    }
}

/** A piece done: a tile made, or a brick lit (its light in its tile). */
type Made = (usize, Piece, Option<Arc<Tile>>);

pub struct World {
    tiles: Vec<HashMap<TileId, Arc<Tile>>>,
    /** Per level, the tiles made so far, as engines read them. */
    levels: Vec<Arc<Level>>,
    pub heights: Heights,
    /** The threads it makes tiles on, which engines may take as many of. */
    pub threads: usize,
    /** The world its threads make, and the light (if any) it is made into, for Lights of the bakers' own. */
    world: Arc<worldgen::World>,
    shared: Option<Shared>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    done: Receiver<Made>,
    /** Received by wait(), not taken yet. */
    early: Vec<Made>,
    /** With no threads, what works in their place. */
    own: Option<Worker>,
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

/**
 * What makes tiles and lights bricks, on a thread of its own or, a world
 * with no threads, on its caller's: the world all share, and a Light of its own.
 */
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

    /** The most wanted job, if any, now under way, with the tiles made with its own. */
    fn take(&self, q: &mut Queue) -> Option<Taken> {
        let job = q.wanted.pop_front()?;
        q.in_flight.insert((job.0, job.1));
        if let Piece::Tile(id) = job.1 {
            let with = made_with(&self.world, job.0 as u32, id.map(i64::from)).map(|key| key.map(|v| v as i32));
            q.take(job.0, with);
        }
        Some((job, q.shared.clone(), q.lighting, q.lit))
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
                    (level, Piece::Tile(id), Some(Arc::new(Tile::new(id, Arc::new(grid)))))
                };
                grids.into_iter().map(tile).collect()
            }
        }
    }
}

fn work(world: Arc<worldgen::World>, queue: &(Mutex<Queue>, Condvar), send: &Sender<Made>) {
    behind_drawing();
    #[cfg(target_os = "linux")]
    queue.0.lock().expect("queue").clocks.push(own_clock());
    let mut worker = Worker::new(world);
    loop {
        let (gone, taken) = {
            let mut q = queue.0.lock().expect("queue");
            loop {
                let gone = std::mem::take(&mut q.gone);
                match worker.take(&mut q) {
                    None if gone.is_empty() => q = queue.1.wait(q).expect("queue"),
                    taken => break (gone, taken),
                }
            }
        };
        drop(gone);
        for made in taken.into_iter().flat_map(|taken| worker.make(taken)) {
            if send.send(made).is_err() {
                return;
            }
        }
    }
}

impl World {
    /**
     * `world`, generated (into `shared` if given,
     * the light it can be lit by) and indexed on `threads` threads, or
     * with none, where there are none to have (a browser), a piece each
     * time its caller has it work (`work`, `wait`); not lit yet.
     */
    pub fn new(world: worldgen::World, shared: Option<Shared>, threads: usize) -> World {
        let world = Arc::new(world);
        let heights = heights(&world);
        let (send, done) = channel();
        let queue = Arc::new((Mutex::new(Queue { shared: shared.clone(), ..Queue::default() }), Condvar::new()));
        for _ in 0..threads {
            let (world, queue, send) = (world.clone(), queue.clone(), send.clone());
            std::thread::spawn(move || work(world, &queue, &send));
        }
        World {
            tiles: (0..GRID_LEVELS).map(|_| HashMap::new()).collect(),
            levels: (0..GRID_LEVELS).map(|_| Arc::new(Level::new(HashMap::new()))).collect(),
            heights,
            threads,
            own: (threads == 0).then(|| Worker::new(world.clone())),
            world,
            shared,
            queue,
            done,
            early: Vec::new(),
        }
    }

    /** With no threads, makes or lights the piece most wanted, here; whether there was one. */
    pub fn work(&mut self) -> bool {
        let Some(worker) = &mut self.own else { return false };
        let taken = worker.take(&mut self.queue.0.lock().expect("queue"));
        let Some(taken) = taken else { return false };
        self.early.extend(worker.make(taken));
        true
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
        around(level, camera, reach).into_iter().map(|(_, id)| id).filter(|id| !self.tiles[level].contains_key(id)).collect()
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
        {
            let mut q = self.queue.0.lock().expect("queue");
            q.shared = Some(shared);
            q.lighting += 1;
        }
        for (level, tiles) in self.tiles.iter_mut().enumerate() {
            for tile in tiles.values_mut() {
                *tile = Arc::new(tile.unlit());
            }
            self.levels[level] = Arc::new(Level::new(tiles.clone()));
        }
    }

    /** Tiles being made and bricks lit, or waiting to be. */
    pub fn making(&self) -> usize {
        let q = self.queue.0.lock().expect("queue");
        q.wanted.len() + q.in_flight.len()
    }

    /** Whether it lacks a tile `wants` asks for. */
    pub fn lacks(&self, wants: &Wants) -> bool {
        wants.asked.iter().any(|&(level, piece)| matches!(piece, Piece::Tile(id) if !self.tiles[level].contains_key(&id)))
    }

    /** What it owes the view: the tiles to make and the bricks to light, waiting or under way. */
    pub fn owed(&self) -> [usize; 2] {
        let q = self.queue.0.lock().expect("queue");
        let pieces = q.wanted.iter().map(|job| job.1).chain(q.in_flight.iter().map(|job| job.1));
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
            self.queue.0.lock().expect("queue").clocks.iter().map(time).collect()
        }
        #[cfg(not(target_os = "linux"))]
        Vec::new()
    }

    /** Waits until a tile is made or a brick lit. */
    pub fn wait(&mut self) {
        if self.own.is_some() {
            self.work();
            return;
        }
        self.early.extend(self.done.recv());
    }

    /**
     * Takes the tiles made, drops those far from the camera (lattice
     * units), and has done what `wants` asks for, most wanted first: the
     * tiles it names made, and, lit here, the bricks lit, what shows
     * (rays ask for them), small, and in a world that is not flat, little
     * of the tiles made. No other tile is made but with one it names
     * (made_with). Returns whether any level's tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, wants: &Wants) -> bool {
        let Wants { reach, lit, asked } = wants;
        let lit = *lit;
        assert!(lit == Lit::Not || self.shared.is_some(), "a world lit with no light");
        // Tiles are made no farther from the camera than `keep` (in the level's sites), and dropped once farther still.
        let keep = |level: usize| reach[level].unwrap_or(0.0) + KEPT;
        let mut changed = [false; GRID_LEVELS];
        let made: Vec<Made> = std::mem::take(&mut self.early).into_iter().chain(self.done.try_iter()).collect();
        {
            let mut q = self.queue.0.lock().expect("queue");
            for (level, piece, _) in &made {
                q.in_flight.remove(&(*level, *piece));
            }
        }
        let far = |level: usize, id: TileId| distance(level, camera, id) > keep(level) + DROPPED;
        for (level, piece, tile) in made {
            // Made with one asked for, a tile may be far, or here already: that one is kept, with its light.
            if let (Piece::Tile(id), Some(tile)) = (piece, tile)
                && !far(level, id)
                && let std::collections::hash_map::Entry::Vacant(entry) = self.tiles[level].entry(id)
            {
                entry.insert(tile);
                changed[level] = true;
            }
        }
        for (level, tiles) in self.tiles.iter_mut().enumerate() {
            let before = tiles.len();
            tiles.retain(|&id, _| !far(level, id));
            changed[level] |= tiles.len() < before;
        }
        // What the light forgot is long to free: a thread's to, if the world has any.
        let gone = self.shared.as_ref().and_then(|shared| shared.forget(|level, key| far(level, key.map(|k| k as i32))));
        let gone = gone.filter(|_| self.threads > 0);

        let (mut list, mut listed) = (VecDeque::new(), HashSet::new());
        let unlit = |tile: &Tile, [x, y, z]: [i32; 3]| usize::try_from(tile.grid.brick(x, y, z)).is_ok_and(|f| tile.light(f).is_none());
        for &(level, piece) in asked.iter() {
            let job = match piece {
                Piece::Tile(id) => (distance(level, camera, id) <= keep(level) && !self.tiles[level].contains_key(&id)).then_some(None),
                Piece::Brick(corner) => {
                    let tile = self.tiles[level].get(&corner.map(|v| v.div_euclid(TILE as i32)));
                    tile.filter(|t| lit == Lit::Bricks && unlit(t, corner)).map(|t| Some(t.clone()))
                }
            };
            if let Some(of) = job
                && listed.insert((level, piece))
            {
                list.push_back((level, piece, of));
            }
        }
        {
            let mut q = self.queue.0.lock().expect("queue");
            q.lit = lit != Lit::Not;
            list.retain(|job| !q.in_flight.contains(&(job.0, job.1)));
            q.gone.extend(gone);
            if !list.is_empty() || !q.gone.is_empty() {
                self.queue.1.notify_all();
            }
            q.wanted = list;
        }

        for level in (0..GRID_LEVELS).filter(|&l| changed[l]) {
            self.levels[level] = Arc::new(Level::new(self.tiles[level].clone()));
        }
        changed.contains(&true)
    }
}

/** For tests: the tiles of `level` of `world`, unlit, that meet the cube reaching `reach` sites each way from `centre`. */
#[cfg(test)]
pub fn tiles_about(world: &worldgen::World, level: u32, centre: [i32; 3], reach: i32) -> HashMap<TileId, Arc<Tile>> {
    let tile = |v: i32| v.div_euclid(TILE as i32);
    let mut tiles = HashMap::new();
    for id in within([centre.map(|v| tile(v - reach)), centre.map(|v| tile(v + reach - 1))]) {
        if !tiles.contains_key(&id) {
            let made = make_tiles(world, level, id.map(i64::from)).into_iter().map(|(key, grid)| (key.map(|v| v as i32), grid));
            tiles.extend(made.map(|(id, grid)| (id, Arc::new(Tile::new(id, Arc::new(grid))))));
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
            let tiles = world.level(coarsest).tiles.values();
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
        let light = |t: &Tile| -> Vec<_> { (0..t.grid.cells.len() / fcc::BRICK_SITES).map(|f| t.light(f).map(<[u32]>::to_vec)).collect() };
        let mut lights: Vec<_> = world.level(coarsest).tiles.iter().map(|(&id, t)| (id, light(t))).collect();
        lights.sort();
        assert!(lights.iter().any(|(_, l)| l.iter().any(Option::is_some)), "some light found");
        lights
    }

    /** Where the sites of each brick of `grid` with voxels start. */
    fn corners(grid: &Grid) -> impl Iterator<Item = [i32; 3]> + '_ {
        let count = grid.n.map(|v| (v + fcc::BRICK - 1) / fcc::BRICK);
        (grid.bricks.iter().enumerate()).filter(|(_, first)| **first >= 0).map(move |(place, _)| {
            let b = place as i32;
            let at = [b % count[0], b / count[0] % count[1], b / count[0] / count[1]];
            std::array::from_fn(|d| grid.min[d] + at[d] * fcc::BRICK)
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

    /**
     * Level::meets answers as reading each site would, about a world's spawn
     * at a level: yes for each cell holding a voxel, or a see-through one,
     * and no for each other of 8·FCC, 32·FCC or smaller.
     */
    #[test]
    fn cells_meet_the_voxels_they_hold() {
        // Around the corner of cubes this many sites a side that is nearest the spawn, as far each way.
        const REACH: i32 = 128;
        let mut see_through = 0;
        for (name, level) in [("city", 0), ("moor", 0), ("forest", 2)] {
            let world = (worlds::by_name(name).expect("world").make)(1);
            let spawn = world.spawn.map(|v| (v >> level) as i32);
            let corner = spawn.map(|v| (v as f64 / REACH as f64).round() as i32 * REACH);
            let level = Level::new(tiles_about(&world, level, corner, REACH));
            for kept in [false, true] {
                let voxel = |look: fcc::Look| if kept { !matches!(fcc::alpha_of(look), 0 | fcc::ALPHA) } else { look != 0 };
                let held = |q: [i32; 3]| (q[0] + q[1] + q[2]) & 1 == 0 && level.grids.at(q[0], q[1], q[2]).is_some_and(voxel);
                for radius in [1, 2, 4, 8, 16, 32] {
                    // Each cell within REACH of the spawn (to the next multiple of 64, a site of each), or 8 cells, the nearer.
                    let n = (REACH / radius).min(8);
                    let cells: Vec<[i32; 3]> = within([[-n; 3], [n; 3]])
                        .filter(|p| (p[0] + p[1] + p[2]) & 1 == 0)
                        .map(|p| std::array::from_fn(|d| (spawn[d] + 32).div_euclid(64) * 64 + p[d] * radius))
                        .collect();
                    let mut holding = 0;
                    for &c in &cells {
                        let inside = |q: [i32; 3]| (0..3).all(|d| (q[d] - c[d]).abs() + (q[(d + 1) % 3] - c[(d + 1) % 3]).abs() <= radius);
                        let holds = within([c.map(|v| v - radius), c.map(|v| v + radius)]).any(|q| inside(q) && held(q));
                        let meets = level.meets(c, radius, kept);
                        assert!(meets || !holds, "{name}: a cell of {radius} about {c:?} holds a voxel (see-through: {kept})");
                        assert!(!meets || holds || radius == 16, "{name}: a cell of {radius} about {c:?} holds none (see-through: {kept})");
                        holding += usize::from(holds);
                    }
                    if kept {
                        see_through += holding;
                    } else {
                        assert!(
                            holding > 0 && holding < cells.len(),
                            "{name}, cells of {radius}: {holding} of {} hold voxels",
                            cells.len()
                        );
                    }
                }
            }
        }
        assert!(see_through > 0, "some cells hold see-through voxels");
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
    fn a_world_with_no_threads_is_as_one_with_some() {
        use light::Brightness;
        let make = || (worlds::by_name("forest").expect("forest").make)(1);
        let day = Brightness { sky: 1.0, sun: 1.0, glow: 0.0 };
        assert!(lit(&mut World::new(make(), Some(Shared::new(day)), 0)) == lit(&mut World::new(make(), Some(Shared::new(day)), 4)));
    }
}
