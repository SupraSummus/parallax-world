//! A generated world's tiles: each level's voxels are tiles fixed in the
//! world, cubes TILE sites of the level a side (worldgen's tile_box), no
//! axis apart from the others: asked for, made and dropped by how far they
//! are from the camera, as the crow flies. Of each, only what may show is
//! generated (worldgen's make_tiles): none of it above or below the world's
//! heights, nor where its layers say it is empty or buried; on the
//! world's threads (a generator per thread), the tiles wanted one above
//! another together if the world has heights, and indexed there.
//! Which tiles, and in what order, is up to the engine drawing them (Wants): every update hands over the tiles it misses for
//! what is in view, most wanted first, replacing the previous list; then
//! the rest of each level it reads, as far around the camera as it reads
//! it whichever way it looks (Wants's reach), so that turning finds them made. Tiles far from the camera are dropped, and
//! the light's blocks in them (worldgen's Shared). A tile
//! never changes, so a chunk baked once all the tiles it reads are in
//! stays right whatever comes after (fcc's Tiles). Given light (worldgen's Shared),
//! the bakers' Lights read blocks of their own, each site as it is, where
//! they read them. Asked for light, for the voxels engine
//! (voxels.rs), the bricks of tiles made that its rays asked for are lit,
//! their voxels' light found, before any tile is made: only what shows,
//! which in a world that is not flat is little of all a ball around the
//! eye holds. Lit anew (relight), the tiles are kept, their light found again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use fcc::GRID_LEVELS;
pub use fcc::TileId;
use fcc::{Grid, Tiles};
use worldgen::light::{Light, Shared, make_tiles};

use crate::engine::voxels;
use crate::view::Vec3;

/** Tiles are 2^TILE_SHIFT sites of their level wide. */
pub const TILE_SHIFT: u32 = 7;
pub const TILE: i64 = 1 << TILE_SHIFT;
// Lit, tiles are the Lights'.
const _: () = assert!(TILE == worldgen::light::TILE);

/** The world of a seed. */
pub type Make = fn(u64) -> worldgen::World;

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
 * camera as it reads them whichever way it looks, `reach`, in the level's
 * sites (none if it reads none), lit as `lit` says; the tiles it lacks for
 * what is in view, most wanted first; and, lit here, the bricks of tiles
 * made to light, by level and where their sites start, most wanted first.
 */
pub struct Wants {
    pub reach: [Option<f64>; GRID_LEVELS],
    pub lit: Lit,
    pub tiles: Vec<(usize, TileId)>,
    pub bricks: Arc<Bricks>,
}

/** Bricks, by level and where their sites start. */
pub type Bricks = Vec<(usize, [i32; 3])>;

/** What a worker does: makes a tile, or lights a brick of one made, where its sites start. */
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Piece {
    Tile(TileId),
    Brick([i32; 3]),
}

/** How many of the most wanted pieces a worker picks its next brick from (Queue::next). */
const NEAR: usize = 64;

/** A piece of a level, and the tile a brick is of. */
type Job = (usize, Piece, Option<Arc<Tile>>);

/** The pieces to make or light, most wanted first, those being made or lit, and the light they are lit by: how many times it changed. */
#[derive(Default)]
struct Queue {
    wanted: VecDeque<Job>,
    in_flight: HashSet<(usize, Piece)>,
    shared: Option<Shared>,
    lighting: u32,
}

impl Queue {
    /**
     * The piece a worker is to do next, whose last was `last` if a brick:
     * of the NEAR most wanted, the brick of that level nearest it, else the
     * most wanted. Its Light keeps the sites it read last, which bricks
     * side by side read again; taken in turn, each worker's would lie far
     * apart, all around the eye as they are wanted.
     */
    fn next(&mut self, last: Option<(usize, [i32; 3])>) -> Option<Job> {
        let apart = |a: [i32; 3], b: [i32; 3]| (0..3).map(|d| (i64::from(a[d]) - i64::from(b[d])).pow(2)).sum::<i64>();
        let near = last.and_then(|(level, at)| {
            let bricks = self.wanted.iter().take(NEAR).enumerate();
            bricks
                .filter_map(|(k, job)| match job.1 {
                    Piece::Brick(c) if job.0 == level => Some((apart(c, at), k)),
                    _ => None,
                })
                .min()
        });
        self.wanted.remove(near.map_or(0, |(_, k)| k))
    }

    /** Takes the tiles wanted right below and right above tile `id` of `level`, to be made with it (make_tiles): how many below, and above. */
    fn column(&mut self, level: usize, id: TileId) -> [i32; 2] {
        [-1, 1].map(|way| {
            let mut taken = 0;
            loop {
                let next = (level, Piece::Tile([id[0], id[1] + way * (taken + 1), id[2]]));
                let Some(at) = self.wanted.iter().position(|job| (job.0, job.1) == next) else { return taken };
                self.wanted.remove(at);
                self.in_flight.insert(next);
                taken += 1;
            }
        })
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
    /** The world and the light (if any) it is made into, for Lights of the bakers' own. */
    make: Make,
    seed: u64,
    shared: Option<Shared>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    done: Receiver<Made>,
    /** Received by wait(), not taken yet. */
    early: Vec<Made>,
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

fn work(make: Make, seed: u64, heights: &Heights, queue: &(Mutex<Queue>, Condvar), send: &Sender<Made>) {
    behind_drawing();
    // Made on the thread: a generator is not Send. Its Light's too: what tiles and light's blocks share is planned once.
    let world = Rc::new(make(seed));
    // A world with heights lies across y, so tiles one above another share most of their cost; an endless one's
    // (the cheese) cost as much together as alone, and take a box that large.
    let columns = world.heights.is_some();
    // Made once there are tiles to light, and again when the light changes.
    let mut light: Option<(u32, Light)> = None;
    // The brick it lit last (Queue::next).
    let mut lit = None;
    loop {
        let ((level, piece, of), [below, above], shared, lighting) = {
            let mut q = queue.0.lock().expect("queue");
            loop {
                if let Some(job) = q.next(lit) {
                    q.in_flight.insert((job.0, job.1));
                    let column = match job.1 {
                        Piece::Tile(id) if columns => q.column(job.0, id),
                        _ => [0, 0],
                    };
                    break (job, column, q.shared.clone(), q.lighting);
                }
                q = queue.1.wait(q).expect("queue");
            }
        };
        let made: Vec<Made> = match piece {
            Piece::Brick(corner) => {
                lit = Some((level, corner));
                light.take_if(|(l, _)| *l != lighting);
                let (_, light) =
                    light.get_or_insert_with(|| (lighting, Light::new(world.clone(), shared.expect("light to light tiles with"))));
                // A tile lit anew since (relight) is let go, and this light with it.
                of.expect("the tile of a brick").lit(level as u32, corner, light);
                vec![(level, piece, None)]
            }
            Piece::Tile([x, y, z]) => {
                let (first, last) = ([x, y - below, z], y + above);
                let grids = make_tiles(&world.generator, level as u32, heights[level], first.map(i64::from), last.into());
                let tile = |(y, grid)| (level, Piece::Tile([x, y, z]), Some(Arc::new(Tile::new([x, y, z], Arc::new(grid)))));
                (first[1]..=last).zip(grids).map(tile).collect()
            }
        };
        for made in made {
            if send.send(made).is_err() {
                return;
            }
        }
    }
}

impl World {
    /** The world `make` makes of `seed`, generated (into `shared` if given, the light it can be lit by) and indexed on `threads` threads; not lit yet. */
    pub fn new(make: Make, seed: u64, shared: Option<Shared>, threads: usize) -> World {
        let world = make(seed);
        let heights = heights(&world);
        let (send, done) = channel();
        let queue = Arc::new((Mutex::new(Queue { shared: shared.clone(), ..Queue::default() }), Condvar::new()));
        for _ in 0..threads {
            let (queue, send) = (queue.clone(), send.clone());
            std::thread::spawn(move || work(make, seed, &heights, &queue, &send));
        }
        World {
            tiles: (0..GRID_LEVELS).map(|_| HashMap::new()).collect(),
            levels: (0..GRID_LEVELS).map(|_| Arc::new(Level::new(HashMap::new()))).collect(),
            heights,
            threads,
            make,
            seed,
            shared,
            queue,
            done,
            early: Vec::new(),
        }
    }

    /** The tiles of `level` made so far. */
    pub fn level(&self, level: usize) -> &Arc<Level> {
        &self.levels[level]
    }

    /** A Light of the world for each who asks, if it was given light: it lights what they read from the tiles' light. */
    pub fn light(&self) -> Option<impl Fn() -> Light + Send + Sync + 'static> {
        let (make, seed) = (self.make, self.seed);
        self.shared.clone().map(|shared| move || Light::new(make(seed), shared.clone()))
    }

    /**
     * The tiles of each level within its `reach` of the camera, in its
     * sites, not made yet: what an engine that reads all around the eye
     * lacks, level by level, nearest first.
     */
    pub fn lacking(&self, reach: &[Option<f64>; GRID_LEVELS], camera: Vec3) -> Vec<(usize, TileId)> {
        (0..GRID_LEVELS)
            .flat_map(|level| reach[level].map(|r| (level, around(level, camera, r))))
            .flat_map(|(level, around)| {
                around.into_iter().filter(move |(_, id)| !self.tiles[level].contains_key(id)).map(move |(_, id)| (level, id))
            })
            .collect()
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

    /** Makes no more tiles but those being made. */
    pub fn stop(&self) {
        self.queue.0.lock().expect("queue").wanted.clear();
    }

    /** Tiles being made and bricks lit, or waiting to be. */
    pub fn making(&self) -> usize {
        let q = self.queue.0.lock().expect("queue");
        q.wanted.len() + q.in_flight.len()
    }

    /** Waits until a tile is made or a brick lit. */
    pub fn wait(&mut self) {
        self.early.extend(self.done.recv());
    }

    /**
     * Takes the tiles made, drops those far from the camera (lattice
     * units), and, lit here, has the bricks `wants` names lit first: what
     * shows (rays ask for them), small, and in a world that is not flat,
     * little of the tiles made. Then has the tiles it names made, most
     * wanted first, then the rest within its reach of each level. Returns
     * whether any level's tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, wants: &Wants) -> bool {
        let Wants { reach, lit, tiles: wanted, bricks } = wants;
        let lit = *lit;
        assert!(lit == Lit::Not || self.shared.is_some(), "a world lit with no light");
        // Tiles are made no farther from the camera than `keep` (in the level's sites), and dropped once a tile farther.
        let keep = |level: usize| reach[level].unwrap_or(0.0) + 2.0 * TILE as f64;
        let mut changed = [false; GRID_LEVELS];
        let made: Vec<Made> = std::mem::take(&mut self.early).into_iter().chain(self.done.try_iter()).collect();
        {
            let mut q = self.queue.0.lock().expect("queue");
            for (level, piece, _) in &made {
                q.in_flight.remove(&(*level, *piece));
            }
        }
        for (level, piece, tile) in made {
            if let (Piece::Tile(id), Some(tile)) = (piece, tile) {
                self.tiles[level].insert(id, tile);
                changed[level] = true;
            }
        }
        let far = |level: usize, id: TileId| distance(level, camera, id) > keep(level) + TILE as f64;
        for (level, tiles) in self.tiles.iter_mut().enumerate() {
            let before = tiles.len();
            tiles.retain(|&id, _| !far(level, id));
            changed[level] |= tiles.len() < before;
        }
        if let Some(shared) = &self.shared {
            shared.forget(|level, key| far(level, key.map(|k| k as i32)));
        }

        let mut list = VecDeque::new();
        if lit == Lit::Bricks {
            let tile = |level: usize, corner: [i32; 3]| self.tiles[level].get(&corner.map(|v| v.div_euclid(TILE as i32)));
            let unlit = |tile: &Tile, [x, y, z]: [i32; 3]| usize::try_from(tile.grid.brick(x, y, z)).is_ok_and(|f| tile.light(f).is_none());
            let mut listed = HashSet::new();
            for &(level, corner) in bricks.iter() {
                if let Some(tile) = tile(level, corner).filter(|t| unlit(t, corner))
                    && listed.insert((level, corner))
                {
                    list.push_back((level, Piece::Brick(corner), Some(tile.clone())));
                }
            }
        }
        let mut listed = HashSet::new();
        let mut add = |tiles: &HashMap<TileId, Arc<Tile>>, level: usize, id: TileId| {
            if distance(level, camera, id) <= keep(level) && !tiles.contains_key(&id) && listed.insert((level, id)) {
                list.push_back((level, Piece::Tile(id), None));
            }
        };
        for &(level, id) in wanted.iter() {
            add(&self.tiles[level], level, id);
        }
        for (level, reach) in reach.iter().enumerate() {
            for (_, id) in reach.map(|r| around(level, camera, r)).unwrap_or_default() {
                add(&self.tiles[level], level, id);
            }
        }
        {
            let mut q = self.queue.0.lock().expect("queue");
            list.retain(|job| !q.in_flight.contains(&(job.0, job.1)));
            if !list.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /** The coarsest level's tiles around the origin, by tile, their bricks' light, each brick of voxels asked for, as the voxels engine asks. */
    fn lit(world: &mut World) -> Vec<(TileId, Vec<Option<Vec<u32>>>)> {
        let coarsest = GRID_LEVELS - 1;
        let reach = std::array::from_fn(|l| (l == coarsest).then_some(TILE as f64));
        let mut wants = Wants { reach, lit: Lit::Bricks, tiles: Vec::new(), bricks: Arc::default() };
        loop {
            world.update([0.0; 3], &wants);
            let bricks: Vec<_> = world.level(coarsest).tiles.values().flat_map(|t| corners(&t.grid)).map(|c| (coarsest, c)).collect();
            if bricks.len() != wants.bricks.len() {
                wants.bricks = Arc::new(bricks);
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
        use worldgen::light::make_tile;
        let mut see_through = 0;
        for (name, level) in [("city", 0), ("moor", 0), ("forest", 2)] {
            let world = (worlds::by_name(name).expect("world").make)(1);
            let spawn = world.spawn.map(|v| (v >> level) as i32);
            let corner = spawn.map(|v| (v as f64 / TILE as f64).round() as i32);
            let make = |id: TileId| Tile::new(id, Arc::new(make_tile(&world.generator, level, world.heights_at(level), id.map(i64::from))));
            let tiles = within([corner.map(|v| v - 1), corner]);
            let level = Level::new(tiles.map(|id| (id, Arc::new(make(id)))).collect());
            for kept in [false, true] {
                let voxel = |look: fcc::Look| if kept { !matches!(fcc::alpha_of(look), 0 | fcc::ALPHA) } else { look != 0 };
                let held = |q: [i32; 3]| (q[0] + q[1] + q[2]) & 1 == 0 && level.grids.at(q[0], q[1], q[2]).is_some_and(voxel);
                for radius in [1, 2, 4, 8, 16, 32] {
                    // Each cell within a tile of the spawn (to the next multiple of 64, a site of each), or 8 cells, the nearer.
                    let n = (TILE as i32 / radius).min(8);
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
        use worldgen::light::Brightness;
        let make = worlds::by_name("forest").expect("forest").make;
        let (day, dusk) = (Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }, Brightness { sky: 0.3, sun: 0.1, glow: 1.0 });
        let mut world = World::new(make, 1, Some(Shared::new(day)), 4);
        let before = lit(&mut world);
        world.relight(world.shared().expect("light").under(dusk).stepping(3));
        let after = lit(&mut world);
        assert!(after != before, "the light changed");
        assert!(after == lit(&mut World::new(make, 1, Some(Shared::new(dusk).stepping(3)), 4)));
    }
}
