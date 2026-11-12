//! A generated world's tiles: each level's voxels are tiles fixed in the
//! world, cubes TILE sites of the level a side (worldgen's tile_box), no
//! axis apart from the others: asked for, made and dropped by how far they
//! are from the camera, as the crow flies. Of each, only what may show is
//! generated (worldgen's make_tile): none of it above or below the world's
//! heights, nor where its layers say it is empty or buried; alone on the
//! world's threads (a generator per thread) and indexed there.
//! Which tiles, and in what order, is up to the engine drawing them (Wants): every update hands over the tiles it misses for
//! what is in view, most wanted first, replacing the previous list; then
//! the rest within reach of the camera of each level it reads (GRID_REACH,
//! or as far as the voxels engine's rays read, voxels::READ), so that
//! turning finds them made. Tiles far from the camera are dropped. A tile
//! never changes, so a chunk baked once all the tiles it reads are in
//! stays right whatever comes after (bake's Tiles). Given light (worldgen's Shared),
//! the tiles are those the bakers' Lights read, made once. Asked for light,
//! the light reads every level coarser than a voxel's: the tiles near the
//! camera come first, the coarsest's before all; and, for the voxels engine
//! (voxels.rs), each tile made of a level it reads is lit, its voxels'
//! light found, after every tile to make (lighting takes several times as
//! long), nearest first in each level's own sites. A tile lit is a new
//! tile, in the place of the one made. Lit anew (relight), the tiles are
//! kept, their light found again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

pub use bake::TileId;
use bake::slicing::GRID_LEVELS;
use bake::{Grid, LayerIndex, Tiles};
use worldgen::generator::materials_of;
use worldgen::light::{Brightness, Light, Shared, make_tile};

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

/** A generated tile: its voxels, where they are along each face's layers, and its bricks and, lit, their light as the GPU takes them (voxels.rs). */
pub struct Tile {
    pub grid: Arc<Grid>,
    pub index: LayerIndex,
    pub words: Vec<u32>,
    /** voxels::meets. */
    pub meets: Vec<u32>,
    /** If lit, voxels::lights. */
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

/** How the tiles are lit: not, by what reads them (the bakers' Lights), or each tile here of a level read, once made (the voxels'). */
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lit {
    Not,
    Reading,
    Tiles,
}

/**
 * What an engine reads of the world: the levels whose tiles it reads, as
 * far around the camera as `reach`, in each level's sites, lit as `lit`
 * says; and, first, the tiles it lacks for what is in view, most wanted first.
 */
pub struct Wants {
    pub levels: [bool; GRID_LEVELS],
    pub reach: f64,
    pub lit: Lit,
    pub tiles: Vec<(usize, TileId)>,
}

/** A tile to make, or one made to light. */
type Job = (usize, TileId, Option<Arc<Tile>>);

/** The tiles to make or light, most wanted first, those being made or lit, and the light they are lit by: how many times it changed. */
#[derive(Default)]
struct Queue {
    wanted: VecDeque<Job>,
    in_flight: HashSet<(usize, TileId)>,
    shared: Option<Shared>,
    lighting: u32,
}

/** A tile made or lit, and the queue's `lighting` then. */
type Made = (usize, TileId, u32, Arc<Tile>);

pub struct World {
    tiles: Vec<HashMap<TileId, Arc<Tile>>>,
    /** Per level, the tiles made so far, as engines read them. */
    levels: Vec<Arc<Level>>,
    /** By material id, its colour. */
    pub palette: Arc<Vec<u32>>,
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
    let mut around: Vec<(f64, TileId)> = (lo[2]..=hi[2])
        .flat_map(|k| (lo[1]..=hi[1]).flat_map(move |j| (lo[0]..=hi[0]).map(move |i| [i, j, k])))
        .map(|id| (distance(level, camera, id), id))
        .filter(|(d, _)| *d <= reach)
        .collect();
    around.sort_by(|a, b| a.0.total_cmp(&b.0));
    around
}

fn work(make: Make, seed: u64, heights: &Heights, queue: &(Mutex<Queue>, Condvar), send: &Sender<Made>) {
    // Made on the thread: a generator is not Send.
    let generator = make(seed).generator;
    // Made once there are tiles to light, and again when the light changes.
    let mut light: Option<(u32, Light)> = None;
    loop {
        let ((level, id, made), shared, lighting) = {
            let mut q = queue.0.lock().expect("queue");
            loop {
                if let Some(job) = q.wanted.pop_front() {
                    q.in_flight.insert((job.0, job.1));
                    break (job, q.shared.clone(), q.lighting);
                }
                q = queue.1.wait(q).expect("queue");
            }
        };
        let tile = match made {
            Some(tile) => {
                light.take_if(|(l, _)| *l != lighting);
                let (_, light) =
                    light.get_or_insert_with(|| (lighting, Light::new(make(seed), shared.expect("light to light tiles with"))));
                let lights = Some(voxels::lights(&tile.grid, level as u32, light));
                Tile { grid: tile.grid.clone(), index: tile.index.clone(), words: tile.words.clone(), meets: tile.meets.clone(), lights }
            }
            None => {
                let (heights, key) = (heights[level], id.map(i64::from));
                let grid = match &shared {
                    Some(shared) => shared.tile(&generator, level as u32, heights, key),
                    None => make_tile(&generator, level as u32, heights, key),
                }
                .into_grid(true);
                let index = LayerIndex::new(&grid);
                let (words, meets) = (voxels::words(&grid), voxels::meets(&grid));
                Tile { grid: Arc::new(grid), index, words, meets, lights: None }
            }
        };
        if send.send((level, id, lighting, Arc::new(tile))).is_err() {
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
        let queue = Arc::new((Mutex::new(Queue { shared: shared.clone(), ..Queue::default() }), Condvar::new()));
        for _ in 0..threads {
            let (queue, send) = (queue.clone(), send.clone());
            std::thread::spawn(move || work(make, seed, &heights, &queue, &send));
        }
        World {
            tiles: (0..GRID_LEVELS).map(|_| HashMap::new()).collect(),
            levels: (0..GRID_LEVELS).map(|_| Arc::new(Level::new(HashMap::new()))).collect(),
            palette: Arc::new(materials_of(&world.generator).iter().map(|m| m.rgb()).collect()),
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
     * The tiles of the `levels` read within `reach` of the camera, in each
     * level's sites, not made yet, or not lit yet if `lit` here: what an
     * engine that reads all around the eye lacks, level by level, nearest
     * first.
     */
    pub fn lacking(&self, levels: &[bool; GRID_LEVELS], camera: Vec3, reach: f64, lit: Lit) -> Vec<(usize, TileId)> {
        let lacks = |level: usize, id: &TileId| self.tiles[level].get(id).is_none_or(|t| lit == Lit::Tiles && t.lights.is_none());
        (0..GRID_LEVELS)
            .filter(|&level| levels[level])
            .flat_map(|level| {
                around(level, camera, reach).into_iter().filter(move |(_, id)| lacks(level, id)).map(move |(_, id)| (level, id))
            })
            .collect()
    }

    /** How bright the light it is lit by, if any. */
    pub fn brightness(&self) -> Option<Brightness> {
        self.shared.as_ref().map(Shared::brightness)
    }

    /**
     * Lights the world anew under `brightness`: the tiles made are kept,
     * those lit are to light again, and those being lit are dropped as they
     * arrive. Engines that read its light (Lit) are to be made again.
     */
    pub fn relight(&mut self, brightness: Brightness) {
        let shared = self.shared.as_ref().expect("a world with light").under(brightness);
        self.shared = Some(shared.clone());
        {
            let mut q = self.queue.0.lock().expect("queue");
            q.shared = Some(shared);
            q.lighting += 1;
        }
        for (level, tiles) in self.tiles.iter_mut().enumerate() {
            for tile in tiles.values_mut().filter(|t| t.lights.is_some()) {
                let Tile { grid, index, words, meets, .. } = &**tile;
                *tile =
                    Arc::new(Tile { grid: grid.clone(), index: index.clone(), words: words.clone(), meets: meets.clone(), lights: None });
            }
            self.levels[level] = Arc::new(Level::new(tiles.clone()));
        }
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
     * units), and has the tiles `wants` names made, most wanted first, then
     * the rest within its reach of each level it reads, lit as it wants
     * them; then, lit here, has those of the levels read lit. Returns
     * whether any level's tiles changed.
     */
    pub fn update(&mut self, camera: Vec3, wants: &Wants) -> bool {
        let Wants { levels, reach, lit, tiles: wanted } = wants;
        let (levels, reach, lit) = (*levels, *reach, *lit);
        assert!(lit == Lit::Not || self.shared.is_some(), "a world lit with no light");
        // Tiles are made no farther from the camera than `keep` (in the level's sites), and dropped once a tile farther.
        let keep = reach + 2.0 * TILE as f64;
        let mut changed = [false; GRID_LEVELS];
        let made: Vec<Made> = std::mem::take(&mut self.early).into_iter().chain(self.done.try_iter()).collect();
        let now = {
            let mut q = self.queue.0.lock().expect("queue");
            for (level, id, _, _) in &made {
                q.in_flight.remove(&(*level, *id));
            }
            q.lighting
        };
        for (level, id, lighting, tile) in made {
            // Lit before the light changed.
            if tile.lights.is_some() && lighting != now {
                continue;
            }
            self.tiles[level].insert(id, tile);
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
            if distance(level, camera, id) <= keep && !tiles.contains_key(&id) && listed.insert((level, id)) {
                list.push_back((level, id, None));
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
        for &(level, id) in wanted.iter() {
            add(&self.tiles[level], level, id);
        }
        for level in 0..GRID_LEVELS {
            for (_, id) in reach_of(level).map(|r| around(level, camera, r)).unwrap_or_default() {
                add(&self.tiles[level], level, id);
            }
        }
        // Lit here, after every tile to make, those of the levels read (the Light reads coarser ones' voxels, not their
        // light), nearest first in each level's own sites, the finer first as near: as rays read the levels, so the
        // picture is lit from the eye out, alike at every level. Rays take the next level where a tile is not lit yet.
        if lit == Lit::Tiles {
            let mut lighting: Vec<(f64, Job)> = (0..GRID_LEVELS)
                .filter(|&level| levels[level])
                .flat_map(|level| around(level, camera, reach).into_iter().map(move |(d, id)| (d, level, id)))
                .filter_map(|(d, level, id)| {
                    let tile = self.tiles[level].get(&id).filter(|t| t.lights.is_none())?;
                    Some((d, (level, id, Some(tile.clone()))))
                })
                .collect();
            lighting.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.0.cmp(&b.1.0)));
            list.extend(lighting.into_iter().map(|(_, job)| job));
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

    /** The coarsest level's lights, by tile, of `world` lit around the origin. */
    fn lit(world: &mut World) -> Vec<(TileId, Vec<u32>)> {
        let coarsest = GRID_LEVELS - 1;
        let wants = Wants { levels: std::array::from_fn(|l| l == coarsest), reach: TILE as f64, lit: Lit::Tiles, tiles: Vec::new() };
        loop {
            world.update([0.0; 3], &wants);
            if !world.busy() {
                break;
            }
            world.wait();
        }
        let mut lights: Vec<_> = world.level(coarsest).tiles.iter().map(|(&id, t)| (id, t.lights.clone().expect("lit"))).collect();
        lights.sort();
        lights
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
    fn a_world_lit_anew_is_as_one_made_under_that_brightness() {
        let make = worlds::by_name("forest").expect("forest").make;
        let (day, dusk) = (Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }, Brightness { sky: 0.3, sun: 0.1, glow: 1.0 });
        let mut world = World::new(make, 1, Some(Shared::new(day)), 4);
        let before = lit(&mut world);
        world.relight(dusk);
        let after = lit(&mut world);
        assert!(after != before, "the light changed");
        assert!(after == lit(&mut World::new(make, 1, Some(Shared::new(dusk)), 4)));
    }
}
