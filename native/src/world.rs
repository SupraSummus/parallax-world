//! A generated world's tiles (World in world.ts): each level's voxels are
//! tiles fixed in the world, TILE sites of the level wide along x and z,
//! generated alone on the world's threads (a generator per thread) and
//! indexed there. Which tiles, and in what order, is up to the renderer:
//! every update hands over the tiles it misses for the cells in view, most
//! wanted first, replacing the previous list; then the rest within
//! GRID_REACH of the camera of each level it reads, so that turning
//! finds them made. Tiles far from the camera are dropped. A tile never
//! changes, so a chunk baked once all the tiles it reads are in stays right
//! whatever comes after (bake's Tiles). Lit, the tiles are those the
//! bakers' Lights read (worldgen's Grids), made once.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bake::slicing::{GRID_LEVELS, GRID_REACH};
use bake::{Grid, LayerIndex, Tiles};
use worldgen::generator::generate;
use worldgen::grid::{Box3, compose};
use worldgen::light::Grids;

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
pub fn heights(world: &worldgen::World) -> Heights {
    std::array::from_fn(|level| world.heights_at(level as u32))
}

/** Tile (i, k) holds x in [i · TILE, (i + 1) · TILE), z likewise. */
pub type TileId = (i32, i32);

/** A generated tile: its voxels, and where they are along each face's layers. */
pub struct Tile {
    pub grid: Arc<Grid>,
    pub index: LayerIndex,
}

/** A level's tiles made so far, and their voxels as the bakers read them. */
pub struct Level {
    pub tiles: HashMap<TileId, Arc<Tile>>,
    pub grids: Arc<Tiles>,
}

impl Level {
    pub fn new(tiles: HashMap<TileId, Arc<Tile>>) -> Level {
        let grids = Arc::new(Tiles::new(TILE_SHIFT, tiles.iter().map(|(&id, t)| (id, t.grid.clone()))));
        Level { tiles, grids }
    }

    /** A whole grid of `level` of a world of `heights` cut into tiles, those it has voxels in. */
    pub fn cut(heights: &Heights, level: usize, grid: &Grid) -> Level {
        let tile = |v: i32| (v as i64).div_euclid(TILE) as i32;
        let (from, to) = ([grid.min[0], grid.min[2]].map(tile), [grid.min[0] + grid.n[0] - 1, grid.min[2] + grid.n[2] - 1].map(tile));
        let mut tiles = HashMap::new();
        for k in from[1]..=to[1] {
            for i in from[0]..=to[0] {
                let part = compose(&tile_box(heights, level, (i, k)), &[grid]);
                if part.bricks.iter().any(|&b| b >= 0) {
                    let index = LayerIndex::new(&part);
                    tiles.insert((i, k), Arc::new(Tile { grid: Arc::new(part), index }));
                }
            }
        }
        Level::new(tiles)
    }
}

/** The tiles to make, most wanted first, and those being made. */
#[derive(Default)]
struct Queue {
    wanted: VecDeque<(usize, TileId)>,
    in_flight: HashSet<(usize, TileId)>,
}

type Made = (usize, TileId, Tile);

pub struct World {
    tiles: Vec<HashMap<TileId, Arc<Tile>>>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    done: Receiver<Made>,
    /** Received by wait(), not taken yet. */
    early: Vec<Made>,
}

/** The box of tile `id` of `level`, of a world of `heights`. */
fn tile_box(heights: &Heights, level: usize, (i, k): TileId) -> Box3 {
    let (min_y, max_y) = heights[level];
    let (i, k) = (i as i64, k as i64);
    Box3 { min: [i * TILE, min_y, k * TILE], max: [(i + 1) * TILE, max_y, (k + 1) * TILE] }
}

/** How far tile `id` of `level` is from the camera (lattice units) along x and z, in the level's sites. */
fn distance(level: usize, camera: Vec3, (i, k): TileId) -> f64 {
    let n = (1u64 << level) as f64;
    let off = |v: f64, t: i32| {
        let (lo, hi) = ((t as i64 * TILE) as f64, ((t as i64 + 1) * TILE) as f64);
        (lo - v / n).max(v / n - hi).max(0.0)
    };
    off(camera[0], i).hypot(off(camera[2], k))
}

fn work(make: Make, seed: u64, grids: Option<Grids>, heights: &Heights, queue: &(Mutex<Queue>, Condvar), send: &Sender<Made>) {
    // Made on the thread: a generator is not Send.
    let generator = make(seed).generator;
    loop {
        let (level, id) = {
            let mut q = queue.0.lock().expect("queue");
            loop {
                if let Some(job) = q.wanted.pop_front() {
                    q.in_flight.insert(job);
                    break job;
                }
                q = queue.1.wait(q).expect("queue");
            }
        };
        let grid = match &grids {
            Some(grids) => grids.tile(&generator, level as u32, heights[level], (id.0 as i64, id.1 as i64)),
            None => generate(&generator, &tile_box(heights, level, id), level as u32),
        }
        .into_grid();
        let index = LayerIndex::new(&grid);
        if send.send((level, id, Tile { grid: Arc::new(grid), index })).is_err() {
            return;
        }
    }
}

impl World {
    /** The world `make` makes of `seed`, generated (into `grids` if given) and indexed on `threads` threads. */
    pub fn new(make: Make, seed: u64, grids: Option<Grids>, threads: usize) -> World {
        let world = make(seed);
        let heights = heights(&world);
        let (send, done) = channel();
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        for _ in 0..threads {
            let (queue, send, grids) = (queue.clone(), send.clone(), grids.clone());
            std::thread::spawn(move || work(make, seed, grids, &heights, &queue, &send));
        }
        World { tiles: (0..GRID_LEVELS).map(|_| HashMap::new()).collect(), queue, done, early: Vec::new() }
    }

    /** Makes no more tiles but those being made. */
    pub fn stop(&self) {
        self.queue.0.lock().expect("queue").wanted.clear();
    }

    /** Whether tiles are being made or wait to be. */
    pub fn busy(&self) -> bool {
        let q = self.queue.0.lock().expect("queue");
        !q.wanted.is_empty() || !q.in_flight.is_empty()
    }

    /** Waits until a tile is made. */
    pub fn wait(&mut self) {
        self.early.extend(self.done.recv());
    }

    /**
     * Takes the tiles made, drops those far from the camera (lattice units),
     * and has the `wanted` ones made, most wanted first, then the rest
     * within reach of each of the `levels` read. Returns the levels whose
     * tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, levels: &[bool; GRID_LEVELS], wanted: &[(usize, TileId)]) -> Vec<(usize, Arc<Level>)> {
        // Tiles are made no farther from the camera than `keep` (in the level's sites), and dropped once a tile farther.
        let (reach, keep) = (GRID_REACH, GRID_REACH + 2.0 * TILE as f64);
        let mut changed = [false; GRID_LEVELS];
        let made: Vec<Made> = std::mem::take(&mut self.early).into_iter().chain(self.done.try_iter()).collect();
        {
            let mut q = self.queue.0.lock().expect("queue");
            for (level, id, _) in &made {
                q.in_flight.remove(&(*level, *id));
            }
        }
        for (level, id, tile) in made {
            self.tiles[level].insert(id, Arc::new(tile));
            changed[level] = true;
        }
        for (level, tiles) in self.tiles.iter_mut().enumerate() {
            let before = tiles.len();
            tiles.retain(|&id, _| distance(level, camera, id) <= keep + TILE as f64);
            changed[level] |= tiles.len() < before;
        }

        let mut list = VecDeque::new();
        let mut listed = HashSet::new();
        let mut add = |tiles: &HashMap<TileId, Arc<Tile>>, level: usize, id: TileId| {
            if !tiles.contains_key(&id) && distance(level, camera, id) <= keep && listed.insert((level, id)) {
                list.push_back((level, id));
            }
        };
        for &(level, id) in wanted {
            add(&self.tiles[level], level, id);
        }
        for level in (0..GRID_LEVELS).filter(|&l| levels[l]) {
            let n = (1u64 << level) as f64;
            let tile = |v: f64| (v / n / TILE as f64).floor() as i32;
            let r = (reach / TILE as f64).ceil() as i32;
            let (ci, ck) = (tile(camera[0]), tile(camera[2]));
            let mut around: Vec<(f64, TileId)> = (ck - r..=ck + r)
                .flat_map(|k| (ci - r..=ci + r).map(move |i| (i, k)))
                .map(|id| (distance(level, camera, id), id))
                .filter(|(d, _)| *d <= reach)
                .collect();
            around.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, id) in around {
                add(&self.tiles[level], level, id);
            }
        }
        {
            let mut q = self.queue.0.lock().expect("queue");
            list.retain(|job| !q.in_flight.contains(job));
            if !list.is_empty() {
                self.queue.1.notify_all();
            }
            q.wanted = list;
        }

        (0..GRID_LEVELS).filter(|&l| changed[l]).map(|l| (l, Arc::new(Level::new(self.tiles[l].clone())))).collect()
    }
}
