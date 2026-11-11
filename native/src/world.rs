//! A generated world's tiles (World in world.ts): each level's voxels are
//! tiles fixed in the world, TILE sites of the level wide along x and z,
//! generated alone on the world's threads (a generator per thread) and
//! indexed there. Which tiles, and in what order, is up to the renderer:
//! every update hands over the tiles it misses for the cells in view, most
//! wanted first, replacing the previous list; then the rest within
//! GRID_REACH of the camera of each level it reads, so that turning
//! finds them made. Tiles far from the camera are dropped. A tile never
//! changes, so a chunk baked once all the tiles it reads are in stays right
//! whatever comes after (bake's Tiles). Given light (worldgen's Shared),
//! the tiles are those the bakers' Lights read, made once. Asked for light,
//! the light reads every level coarser than a voxel's: the tiles near the
//! camera come first, the coarsest's before all; and, for the voxels engine
//! (march.rs), each tile made is lit, its voxels' light found, after every
//! tile to make: lighting takes several times as long. A tile lit is a new
//! tile, in the place of the one made.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bake::slicing::{GRID_LEVELS, GRID_REACH};
use bake::{Grid, LayerIndex, Tiles};
use worldgen::generator::generate;
use worldgen::grid::Box3;
use worldgen::light::{Light, Shared};

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

/** A generated tile: its voxels, where they are along each face's layers, and its bricks and, lit, their light as the GPU takes them (march.rs). */
pub struct Tile {
    pub grid: Arc<Grid>,
    pub index: LayerIndex,
    pub words: Vec<u32>,
    /** If lit, march::lights. */
    pub lights: Option<Vec<u32>>,
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
}

/** How the tiles are lit: not, by what reads them (the bakers' Lights), or each tile here, once made (march's). */
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lit {
    Not,
    Reading,
    Tiles,
}

/** A tile to make, or one made to light. */
type Job = (usize, TileId, Option<Arc<Tile>>);

/** The tiles to make or light, most wanted first, and those being made or lit. */
#[derive(Default)]
struct Queue {
    wanted: VecDeque<Job>,
    in_flight: HashSet<(usize, TileId)>,
}

type Made = (usize, TileId, Arc<Tile>);

pub struct World {
    tiles: Vec<HashMap<TileId, Arc<Tile>>>,
    /** How the tiles are lit from the next update on: by the World's light only if it was given some. */
    pub lit: Lit,
    lights: bool,
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

/** The tiles of `level` within `reach` of the camera (lattice units) in the level's sites, nearest first. */
fn around(level: usize, camera: Vec3, reach: f64) -> Vec<(f64, TileId)> {
    let tile = |v: f64| (v / (1u64 << level) as f64 / TILE as f64).floor() as i32;
    let (ci, ck, r) = (tile(camera[0]), tile(camera[2]), (reach / TILE as f64).ceil() as i32);
    let mut around: Vec<(f64, TileId)> = (ck - r..=ck + r)
        .flat_map(|k| (ci - r..=ci + r).map(move |i| (i, k)))
        .map(|id| (distance(level, camera, id), id))
        .filter(|(d, _)| *d <= reach)
        .collect();
    around.sort_by(|a, b| a.0.total_cmp(&b.0));
    around
}

fn work(make: Make, seed: u64, shared: Option<Shared>, heights: &Heights, queue: &(Mutex<Queue>, Condvar), send: &Sender<Made>) {
    // Made on the thread: a generator is not Send.
    let generator = make(seed).generator;
    let mut light = shared.clone().map(|shared| Light::new(make(seed), shared));
    loop {
        let (level, id, made) = {
            let mut q = queue.0.lock().expect("queue");
            loop {
                if let Some(job) = q.wanted.pop_front() {
                    q.in_flight.insert((job.0, job.1));
                    break job;
                }
                q = queue.1.wait(q).expect("queue");
            }
        };
        let tile = match made {
            Some(tile) => {
                let light = light.as_mut().expect("light to light tiles with");
                let lights = Some(crate::march::lights(&tile.grid, level as u32, light));
                Tile { grid: tile.grid.clone(), index: tile.index.clone(), words: tile.words.clone(), lights }
            }
            None => {
                let grid = match &shared {
                    Some(shared) => shared.tile(&generator, level as u32, heights[level], (id.0 as i64, id.1 as i64)),
                    None => generate(&generator, &tile_box(heights, level, id), level as u32),
                }
                .into_grid(true);
                let index = LayerIndex::new(&grid);
                let words = crate::march::words(&grid);
                Tile { grid: Arc::new(grid), index, words, lights: None }
            }
        };
        if send.send((level, id, Arc::new(tile))).is_err() {
            return;
        }
    }
}

impl World {
    /** The world `make` makes of `seed`, generated (into `shared` if given, the light it can be lit by) and indexed on `threads` threads; not lit yet. */
    pub fn new(make: Make, seed: u64, shared: Option<Shared>, threads: usize) -> World {
        let world = make(seed);
        let heights = heights(&world);
        let (send, done) = channel();
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        for _ in 0..threads {
            let (queue, send, shared) = (queue.clone(), send.clone(), shared.clone());
            std::thread::spawn(move || work(make, seed, shared, &heights, &queue, &send));
        }
        let lights = shared.is_some();
        World { tiles: (0..GRID_LEVELS).map(|_| HashMap::new()).collect(), lit: Lit::Not, lights, queue, done, early: Vec::new() }
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
     * Takes the tiles made or lit, drops those far from the camera (lattice
     * units), and has the `wanted` ones made, most wanted first, then the
     * rest within reach of each of the `levels` read, as `lit` wants them;
     * then, lit here, has those lit. Returns the levels whose tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, levels: &[bool; GRID_LEVELS], wanted: &[(usize, TileId)]) -> Vec<(usize, Arc<Level>)> {
        let lit = self.lit;
        assert!(lit == Lit::Not || self.lights, "a world lit with no light");
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
            self.tiles[level].insert(id, tile);
            changed[level] = true;
        }
        for (level, tiles) in self.tiles.iter_mut().enumerate() {
            let before = tiles.len();
            tiles.retain(|&id, _| distance(level, camera, id) <= keep + TILE as f64);
            changed[level] |= tiles.len() < before;
        }

        let (mut list, mut lighting) = (VecDeque::new(), VecDeque::new());
        let mut listed = HashSet::new();
        let mut add = |tiles: &HashMap<TileId, Arc<Tile>>, level: usize, id: TileId| {
            if distance(level, camera, id) <= keep && listed.insert((level, id)) {
                match tiles.get(&id) {
                    None => list.push_back((level, id, None)),
                    Some(tile) if lit == Lit::Tiles && tile.lights.is_none() => lighting.push_back((level, id, Some(tile.clone()))),
                    Some(_) => {}
                }
            }
        };
        // Lit, every voxel's light comes from each coarser level's around it (worldgen's light.rs): the tiles of every
        // level near the camera first, the coarsest's, which take longest, before all; then those of the levels coarser
        // than any read as far as the coarsest read reaches, at their sites' size.
        let coarsest = (0..GRID_LEVELS).rev().find(|&l| levels[l]);
        let reach_of = |level: usize| match coarsest {
            Some(c) if lit != Lit::Not && level > c => Some(reach / (1u64 << (level - c)) as f64),
            _ => levels[level].then_some(reach),
        };
        if lit != Lit::Not {
            for level in (0..GRID_LEVELS).rev() {
                for (_, id) in around(level, camera, TILE as f64 / 2.0) {
                    add(&self.tiles[level], level, id);
                }
            }
        }
        for &(level, id) in wanted {
            add(&self.tiles[level], level, id);
        }
        for level in 0..GRID_LEVELS {
            for (_, id) in reach_of(level).map(|r| around(level, camera, r)).unwrap_or_default() {
                add(&self.tiles[level], level, id);
            }
        }
        {
            let mut q = self.queue.0.lock().expect("queue");
            list.append(&mut lighting);
            list.retain(|job| !q.in_flight.contains(&(job.0, job.1)));
            if !list.is_empty() {
                self.queue.1.notify_all();
            }
            q.wanted = list;
        }

        (0..GRID_LEVELS).filter(|&l| changed[l]).map(|l| (l, Arc::new(Level::new(self.tiles[l].clone())))).collect()
    }
}
