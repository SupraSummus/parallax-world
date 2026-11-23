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
//!
//! A world that moves (worldgen's time.rs, a prototype) is made at the
//! world's time, `now`: a tile says while when it stays as made, and at
//! another time it is made anew, alone, and shown till the new one takes its place.

#[cfg(test)]
use std::collections::HashMap;
use std::collections::{BTreeSet, VecDeque};
use std::ops::Range;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, OnceLock};

use fcc::GRID_LEVELS;
pub use fcc::TileId;
use fcc::{Blocks, FastSet, Grid, Tiles};
use light::{Light, Shared};
use worldgen::generator::Drew;
use worldgen::tiles::{made_with, make_tile, make_tiles};
use worldgen::time;

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
    /** While when it stays as made, the world's time (worldgen's time.rs): always, unless what it holds moves. */
    pub stays: Range<i64>,
    /** The layers that drew in it or read the time, if known (worldgen's `Drew`): made anew, it is made of only them. */
    drew: Option<Drew>,
}

/**
 * A level's tiles made so far, and their voxels as the bakers read them:
 * as they were when it was taken. Both are held in blocks (fcc's Blocks),
 * so that the next, some tiles come or gone, costs as those do.
 */
#[derive(Clone)]
pub struct Level {
    pub tiles: Blocks<Arc<Tile>>,
    pub grids: Arc<Tiles>,
}

impl Tile {
    /** Tile `id`, unlit, as made always. */
    pub fn new(id: TileId, grid: Arc<Grid>) -> Tile {
        let meets = voxels::meets(&grid);
        let met = voxels::Met::new(id, &grid, &meets);
        Tile { whole: whole(&grid), lights: unlit(&grid), grid, meets, met, stays: time::ALWAYS, drew: None }
    }

    /** Tile `self` with none of its light found. */
    pub fn unlit(&self) -> Tile {
        let (grid, meets, met, whole) = (self.grid.clone(), self.meets.clone(), self.met.clone(), self.whole.clone());
        Tile { lights: unlit(&grid), grid, meets, met, whole, stays: self.stays.clone(), drew: self.drew.clone() }
    }

    /** The light of the brick whose sites start at `first` in the grid's cells (in bricks), if found. */
    pub fn light(&self, first: usize) -> Option<&[u32]> {
        self.lights[first].get().map(|l| &l[..])
    }

    /**
     * Tile `self`, of `level`, made anew in place of `old` (World::now),
     * with the light `old` found: a brick whose sites are as they were
     * takes it, one changed is lit by `light`. What showed of `old` so
     * shows of it at once, rays not waiting for its light.
     */
    fn lit_as(self, old: &Tile, level: u32, light: &mut Light) -> Tile {
        for (_, b, first) in voxels::bricks(&self.grid) {
            let corner = b.map(|v| v * fcc::BRICK);
            let Ok(had) = usize::try_from(old.grid.brick(corner[0], corner[1], corner[2])) else { continue };
            let Some(found) = old.light(had) else { continue };
            let sites = |first: usize| first * fcc::BRICK_SITES..(first + 1) * fcc::BRICK_SITES;
            if self.grid.cells[sites(first)] == old.grid.cells[sites(had)] {
                _ = self.lights[first].set(found.into());
            } else {
                self.lit(level, corner, light);
            }
        }
        self
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
    pub fn new(tiles: impl IntoIterator<Item = (TileId, Arc<Tile>)>) -> Level {
        let tiles: Blocks<Arc<Tile>> = tiles.into_iter().collect();
        let grids = Arc::new(Tiles::new(TILE_SHIFT, tiles.iter().map(|(id, t)| (id, t.grid.clone()))));
        Level { tiles, grids }
    }

    /** Takes `tile` as its tile `id`: its voxels are others' than those read so far (`grids`). The one it had there, if any. */
    fn insert(&mut self, id: TileId, tile: Arc<Tile>) -> Option<Arc<Tile>> {
        Arc::make_mut(&mut self.grids).insert(id, tile.grid.clone());
        self.tiles.insert(id, tile)
    }

    /** Lets its tile `id` go, if it has it. */
    fn remove(&mut self, id: TileId) -> Option<Arc<Tile>> {
        let tile = self.tiles.remove(id)?;
        Arc::make_mut(&mut self.grids).remove(id);
        Some(tile)
    }

    /**
     * Whether a voxel of its tiles, or (`see_through`) one of a medium,
     * meets the cell of `radius`·FCC about `centre`, a rhombic dodecahedron
     * as a voxel is of radius 1.
     * A cell smaller than those of 8·FCC is read site by site;
     * a larger one asks the cells of 8·FCC or 32·FCC that meet it (voxels::Met),
     * so of those sizes it is exact, and of the others it may say yes wrongly.
     */
    pub fn meets(&self, centre: [i32; 3], radius: i32, see_through: bool) -> bool {
        if radius < voxels::cell_size(0) {
            let kept = |look| if see_through { fcc::medium(look) } else { look != 0 };
            return meeting(centre, radius, 1).any(|q| self.grids.at(q[0], q[1], q[2]).is_some_and(kept));
        }
        let j = usize::from(radius >= voxels::cell_size(1));
        meeting(centre, radius, voxels::cell_size(j))
            .any(|k| within(voxels::Met::tiles(j, k)).any(|t| self.tiles.get(t).is_some_and(|tile| tile.met.has(t, j, k, see_through))))
    }

    /**
     * Whether every site from `lo` to `hi`, inclusive, holds a whole voxel:
     * whether all the bricks holding them are whole (Tile's `whole`), all
     * their tiles made. So it may say no wrongly, never yes.
     */
    pub fn buried(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        let tile = |v: i32| (v as i64).div_euclid(TILE) as i32;
        within([lo.map(tile), hi.map(tile)]).all(|id| {
            let Some(t) = self.tiles.get(id) else { return false };
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
 * lacks for what is in view, and, lit here, the bricks of tiles made to light;
 * and what it sees, if not all around (Sight): a tile of another time it does
 * not see waits to be made anew till it does.
 */
pub struct Wants {
    pub reach: [Option<f64>; GRID_LEVELS],
    pub lit: Lit,
    pub asked: Arc<Vec<(usize, Piece)>>,
    pub sight: Option<Sight>,
}

/** What the camera sees: rays as far off `forward` (a unit vector) as `half` (radians). */
#[derive(Clone, Copy, Debug)]
pub struct Sight {
    pub forward: Vec3,
    pub half: f64,
}

impl Sight {
    /** Whether a ray from the camera may meet tile `id` of `level`: whether its ball meets the cone. */
    fn sees(&self, level: usize, camera: Vec3, id: TileId) -> bool {
        let side = (TILE << level) as f64;
        let to: Vec3 = std::array::from_fn(|d| (id[d] as f64 + 0.5) * side - camera[d]);
        let (far, radius) = (crate::view::dot(to, to).sqrt(), side * 3f64.sqrt() / 2.0);
        far <= radius || (crate::view::dot(to, self.forward) / far).clamp(-1.0, 1.0).acos() <= self.half + (radius / far).asin()
    }
}

/** What a worker does, of a level: makes a tile, or lights a brick of one made, where its sites start. */
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Piece {
    Tile(TileId),
    Brick([i32; 3]),
}

/** A piece of a level, and the tile a brick is of, or the one a tile made anew takes the place of. */
type Job = (usize, Piece, Option<Arc<Tile>>);

/** What the thread that draws has the workers do, in turn. */
enum Order {
    /**
     * Make or light `wanted`, most wanted first, in place of what was
     * wanted, but what is under way; into the light if `lit` (Wants's);
     * at the world's time `now`; free what is `gone`; and forget the
     * light of `level` far from `camera`, past its `keep`.
     */
    Want { wanted: VecDeque<Job>, lit: bool, now: i64, gone: Vec<Box<dyn Send>>, level: usize, camera: Vec3, keep: f64 },
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
 * times it changed, whether tiles are made into it (`lit`), the world's
 * time, what to free, and where the light is to forget from, if asked
 * since it last did.
 */
struct Queue {
    orders: Receiver<Order>,
    now: i64,
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
                Order::Want { mut wanted, lit, now, mut gone, level, camera, keep } => {
                    wanted.retain(|job| !self.in_flight.contains(&(job.0, job.1)));
                    self.gone.append(&mut gone);
                    (self.wanted, self.lit, self.now, self.forget) = (wanted, lit, now, Some((level, camera, keep)));
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

/** A tile of a level, and the world's time it stops staying as made at (`Moving`). */
type Due = (i64, usize, TileId);

/**
 * The tiles made that hold what moves, by level: by the time each stays
 * as made till, and from. So those made for another time than the world's
 * are found as many as they are, however many move, on in time as back.
 */
#[derive(Default)]
struct Moving {
    ends: BTreeSet<Due>,
    starts: BTreeSet<Due>,
}

impl Moving {
    /** Takes tile `id` of `level`, staying as made while `stays`: if not always, it moves. */
    fn add(&mut self, level: usize, id: TileId, stays: &Range<i64>) {
        if *stays != time::ALWAYS {
            self.ends.insert((stays.end, level, id));
            self.starts.insert((stays.start, level, id));
        }
    }

    /** Lets tile `id` of `level` go, as `add` took it. */
    fn remove(&mut self, level: usize, id: TileId, stays: &Range<i64>) {
        self.ends.remove(&(stays.end, level, id));
        self.starts.remove(&(stays.start, level, id));
    }

    fn len(&self) -> usize {
        self.ends.len()
    }

    /** Those that do not stay as made at `now`, each with how far behind or ahead of it they are. */
    fn behind(&self, now: i64) -> impl Iterator<Item = (i64, usize, TileId)> + '_ {
        let first = (now + 1, 0, [i32::MIN; 3]);
        let ended = self.ends.range(..first).map(move |&(end, level, id)| (now + 1 - end, level, id));
        let to_come = self.starts.range(first..).map(move |&(start, level, id)| (start - now, level, id));
        ended.chain(to_come)
    }
}

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
    /** The tiles made that hold what moves (Tile's `stays`), by when they stop staying as made. */
    moving: Moving,
    /** Of those, the ones an engine reads and sees made for another time than the world's, as last updated. */
    late: usize,
    pub heights: Heights,
    /** The world's time, milliseconds, as its host sets it: tiles are made at its tick (`tick`), and anew once that is not while they stay. */
    pub now: i64,
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

/**
 * The world's time, ms, its tiles are made at a whole number of: what
 * moves is made anew no more than so often, whatever its pace and level.
 * At the finest levels a cloud crosses a site in a fiftieth of a second,
 * and would be made anew so often that the threads never kept up; on a
 * tick it jumps a few of their sites, near the eye, where they show.
 */
const TICK: i64 = 250;

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

/** A job taken off the queue, and what the queue said then: its light, how many times that changed, whether it is read, and the time. */
type Taken = (Job, Option<Shared>, u32, bool, i64);

impl Worker {
    fn new(world: Arc<worldgen::World>) -> Worker {
        Worker { world, light: None }
    }

    /** The most wanted job, if any, now under way, with the tiles made with its own; and the pieces it took. */
    fn take(&self, q: &mut Queue) -> Option<(Taken, Vec<(usize, Piece)>)> {
        let job = q.wanted.pop_front()?;
        let mut took = vec![(job.0, job.1)];
        // Those wanted that are made with it wait no more. One made anew is made alone: the others made with it may
        // stay as made.
        if let (Piece::Tile(id), None) = (job.1, &job.2) {
            took.extend(made_with(&self.world, job.0 as u32, id.map(i64::from)).map(|key| (job.0, Piece::Tile(key.map(|v| v as i32)))));
            let with: FastSet<_> = took.iter().copied().collect();
            q.wanted.retain(|job| !with.contains(&(job.0, job.1)));
        }
        q.in_flight.extend(took.iter().copied());
        Some(((job, q.shared.clone(), q.lighting, q.lit, q.now), took))
    }

    /** Its Light, by `shared`, as it changed `lighting` times. */
    fn light(&mut self, shared: Option<Shared>, lighting: u32) -> &mut Light {
        self.light.take_if(|(l, _)| *l != lighting);
        let world = &self.world;
        &mut self.light.get_or_insert_with(|| (lighting, Light::new(world.clone(), shared.expect("light to light tiles with")))).1
    }

    fn make(&mut self, ((level, piece, of), shared, lighting, lit, now): Taken) -> Vec<Made> {
        match piece {
            Piece::Brick(corner) => {
                // A tile lit anew since (relight) is let go, and this light with it.
                of.expect("the tile of a brick").lit(level as u32, corner, self.light(shared, lighting));
                vec![(level, piece, None)]
            }
            Piece::Tile(id) if let Some(old) = of => {
                let key = id.map(i64::from);
                let ((grid, stays, drew), _) = time::at(now, || make_tile(&self.world, level as u32, key, old.drew.as_ref()));
                let tile = Tile { stays, drew, ..Tile::new(id, Arc::new(grid)) };
                // Its light found as the old one's was, here: a tile lit anew since has none.
                let tile = if old.lights.iter().any(|l| l.get().is_some()) {
                    tile.lit_as(&old, level as u32, self.light(shared, lighting))
                } else {
                    tile
                };
                vec![(level, piece, Some(Arc::new(tile)))]
            }
            Piece::Tile(id) => {
                let (world, key) = (&self.world, id.map(i64::from));
                // Read by light, its sites are the light's: made once. Each stays as made as long as what may be drawn
                // in it does.
                let (tiles, _) = time::at(now, || match shared.filter(|_| lit) {
                    Some(shared) => shared.make_tiles(world, level as u32, key),
                    None => make_tiles(world, level as u32, key),
                });
                let tile = |(key, grid, stays): worldgen::tiles::Made| {
                    let id = key.map(|v| v as i32);
                    (level, Piece::Tile(id), Some(Arc::new(Tile { stays, ..Tile::new(id, Arc::new(grid)) })))
                };
                tiles.into_iter().map(tile).collect()
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
                now: 0,
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
            moving: Moving::default(),
            late: 0,
            heights,
            now: 0,
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

    /** The time its tiles are made at: `now`, at its tick (TICK). */
    fn tick(&self) -> i64 {
        self.now - self.now.rem_euclid(TICK)
    }

    /** The tiles made that hold what moves, and those of them the engine sees made for another time than the world's, as last updated: behind it. */
    pub fn moving(&self) -> [usize; 2] {
        [self.moving.len(), self.late]
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
     * (made_with); but those made for another time are made anew, if
     * within its reach, else dropped. What it lets go, its threads free:
     * that is long. Returns whether any level's tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, wants: &Wants) -> bool {
        let Wants { reach, lit, asked, sight } = wants;
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
        let now = self.tick();
        let mut gone: Vec<Box<dyn Send>> = Vec::new();
        for (level, piece, tile) in made {
            let (Piece::Tile(id), Some(tile)) = (piece, tile) else { continue };
            // Made with one asked for, a tile may be far, or here already: that one is kept, with its light, unless
            // made for another time than this one.
            if far(level, id) || self.levels[level].tiles.get(id).is_some_and(|had| had.stays.contains(&now)) {
                gone.push(Box::new(tile));
                continue;
            }
            let stays = tile.stays.clone();
            if let Some(had) = Arc::make_mut(&mut self.levels[level]).insert(id, tile) {
                self.moving.remove(level, id, &had.stays);
                gone.push(Box::new(had));
            }
            self.moving.add(level, id, &stays);
            changed = true;
        }
        // Made for another time, a tile no engine reads is dropped: made with one asked for, it would be made anew
        // each step, unread.
        let stale = |level: usize, id: TileId, tile: &Tile| {
            !tile.stays.contains(&now) && reach[level].is_none_or(|reach| distance(level, camera, id) > reach)
        };
        let level = self.sweeping;
        self.sweeping = (level + 1) % GRID_LEVELS;
        let tiles = &self.levels[level].tiles;
        let dropped: Vec<TileId> = tiles.iter().filter(|&(id, tile)| far(level, id) || stale(level, id, tile)).map(|(id, _)| id).collect();
        if !dropped.is_empty() {
            let tiles = Arc::make_mut(&mut self.levels[level]);
            for tile in dropped.into_iter().filter_map(|id| tiles.remove(id).map(|tile| (id, tile))) {
                self.moving.remove(level, tile.0, &tile.1.stays);
                gone.push(Box::new(tile.1));
            }
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
        // Those made for another time within an engine's reach and sight come after all that is asked for, the one
        // furthest from its own first: so all that moves is as far behind, whatever its level, should the threads not
        // keep up. Those it does not see are kept as they are, unseen, till it does.
        let read = |level: usize, id: TileId| reach[level].is_some_and(|reach| distance(level, camera, id) <= reach);
        let seen = |level: usize, id: TileId| sight.is_none_or(|sight| sight.sees(level, camera, id));
        let late: Vec<_> = self.moving.behind(now).filter(|&(_, level, id)| read(level, id) && seen(level, id)).collect();
        self.late = late.len();
        let in_flight = |level: usize, id: TileId| self.in_flight.contains(&(level, Piece::Tile(id)));
        let mut anew: Vec<_> = late.into_iter().filter(|&(_, level, id)| !in_flight(level, id)).collect();
        anew.sort_by_key(|&(behind, ..)| std::cmp::Reverse(behind));
        for (_, level, id) in anew {
            let tile = self.levels[level].tiles.get(id).expect("a tile that moves is held");
            listed.insert((level, Piece::Tile(id)));
            list.push_back((level, Piece::Tile(id), Some(tile.clone())));
        }
        // One taken and done since what was done was read above may be asked for and made again: the second is dropped.
        self.wanted = listed;
        self.orders.send(Order::Want { wanted: list, lit: lit != Lit::Not, now, gone, level, camera, keep: keep(level) });
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
            let made =
                make_tiles(world, level, id.map(i64::from)).into_iter().map(|(key, grid, stays)| (key.map(|v| v as i32), grid, stays));
            tiles.extend(made.map(|(id, grid, stays)| (id, Arc::new(Tile { stays, ..Tile::new(id, Arc::new(grid)) }))));
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
        let mut wants = Wants { reach, lit: Lit::Bricks, asked: Arc::default(), sight: None };
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
        let light = |t: &Tile| -> Vec<_> { (0..t.grid.cells.len() / fcc::BRICK_SITES).map(|f| t.light(f).map(<[u32]>::to_vec)).collect() };
        let mut lights: Vec<_> = world.level(coarsest).tiles.iter().map(|(id, t)| (id, light(t))).collect();
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
                let voxel = |look: fcc::Look| if kept { fcc::medium(look) } else { look != 0 };
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
    fn tiles_of_what_moves_are_made_anew_and_the_rest_kept() {
        // The valley's clouds alone, at level 3: the tiles they are in, and the empty ones under them.
        let (level, eye) = (3, [0.0, 5600.0, 0.0]);
        let reach = std::array::from_fn(|l| (l == level).then_some(TILE as f64));
        let asked = around(level, eye, TILE as f64).into_iter().map(|(_, id)| (level, Piece::Tile(id))).collect();
        let wants = Wants { reach, lit: Lit::Not, asked: Arc::new(asked), sight: None };
        let settle = |world: &mut World, now: i64| {
            world.now = now;
            while world.update(eye, &wants) || world.making() > 0 {
                if world.making() > 0 {
                    world.wait();
                }
            }
            world.level(level).tiles.iter().map(|(id, tile)| (id, tile.clone())).collect::<HashMap<_, _>>()
        };
        let mut world = World::new(worlds::valley::valley(1).alone("clouds"), None, 4);
        // Twice: tiles made one above another stay no longer than the clouds among them; anew, each alone, the empty ones always.
        settle(&mut world, 0);
        let mut before = settle(&mut world, 10_000);
        assert!(before.values().any(|t| t.stays == time::ALWAYS), "what is under the clouds stays");
        // On in time, and back.
        for now in [20_000, 5_000] {
            let after = settle(&mut world, now);
            assert!(world.moving()[0] > 0, "clouds in the tiles");
            for (id, tile) in &before {
                assert_eq!(Arc::ptr_eq(tile, &after[id]), tile.stays == time::ALWAYS, "tile {id:?}: kept if it stays always");
                assert!(after[id].stays.contains(&now));
            }
            before = after;
        }
    }

    /** A tile made anew takes the light of each brick of the old one whose sites are as they were, and lights the rest the old had. */
    #[test]
    fn a_tile_made_anew_is_lit_where_the_old_one_was() {
        use light::Brightness;
        let world = (worlds::by_name("forest").expect("forest").make)(1);
        let tiles = tiles_about(&world, 0, world.spawn.map(|v| v as i32), TILE as i32);
        let (&id, old) = tiles.iter().max_by_key(|(_, t)| voxels::bricks(&t.grid).count()).expect("a tile");
        let mut light = Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
        // Bricks lit up to the second that shows, a site of it that shows emptied.
        let shows = |&(_, b, first): &(usize, [i32; 3], usize)| {
            old.lit(0, b.map(|v| v * fcc::BRICK), &mut light);
            old.light(first).is_some_and(|l| l.iter().any(|&w| w != 0))
        };
        let (_, _, changed) = voxels::bricks(&old.grid).filter(shows).nth(1).expect("two bricks that show");
        let mut grid = (*old.grid).clone();
        let lit = old.light(changed).expect("lit");
        let words = lit.len() / fcc::BRICK_SITES;
        let site = (0..fcc::BRICK_SITES).find(|&k| lit[k * words] != 0).expect("a voxel that shows");
        grid.cells[changed * fcc::BRICK_SITES + site] = 0;
        let new = Tile::new(id, Arc::new(grid)).lit_as(old, 0, &mut light);
        assert!((0..new.lights.len()).filter(|&f| f != changed).all(|f| new.light(f) == old.light(f)), "the rest as it was");
        assert!(new.light(changed).is_some_and(|l| Some(l) != old.light(changed)), "lit anew");
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
