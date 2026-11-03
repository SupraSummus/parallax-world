//! A generated world's grids (World in world.ts): a grid per level of
//! detail, made of tiles fixed in the world, around the camera. When the
//! camera goes far enough from a grid's middle, a grid centred on it is
//! started; its new tiles are generated on the world's thread pool (a forest
//! per thread) and composed with the old grid's off the main thread, with its
//! LayerIndex, and it replaces the old grid once whole.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bake::Grid;
use worldgen::forest::{REACH, forest, heights};
use worldgen::generator::{Generator, generate};
use worldgen::grid::{Box3, compose};

use crate::layer_index::LayerIndex;
use crate::slicing::GRID_LEVELS;
use crate::view::Vec3;

/** Tiles are TILE sites of their level wide along x and z, on a tiling fixed in the world. */
const TILE: i64 = 128;

/** A grid moves when the camera is more than this far from the middle of its centre tile (in the level's sites): past the tile's edge, so it does not move back and forth there. */
const RECENTRE: i64 = TILE * 3 / 4;

/** Tile (i, k) holds x in [i · TILE, (i + 1) · TILE), z likewise. */
type Tile = (i64, i64);

type Lattices = Vec<[[i32; 3]; 3]>;

/** A level's grid, made whole: what the renderer and the bakers read. */
pub struct NewGrid {
    pub level: usize,
    pub grid: Arc<Grid>,
    pub index: LayerIndex,
}

enum Done {
    Part { level: usize, tile: Tile, grid: Grid },
    Grid { centre: Tile, grid: NewGrid },
}

/** A grid being made: its centre tile, the tiles it needs and those generated, until it is composed. */
struct Pending {
    centre: Tile,
    tiles: Vec<Tile>,
    received: Vec<Grid>,
}

pub struct World {
    seed: u64,
    lattices: Lattices,
    pool: Arc<rayon::ThreadPool>,
    grids: Vec<Option<Arc<Grid>>>,
    centres: Vec<Option<Tile>>,
    pending: Vec<Option<Pending>>,
    send: Sender<Done>,
    done: Receiver<Done>,
    /** Received by wait(), not taken yet. */
    early: Vec<Done>,
}

thread_local! {
    /** The forest of the seed last generated on this thread. */
    static FOREST: RefCell<Option<(u64, Generator)>> = const { RefCell::new(None) };
}

fn generate_tile(seed: u64, level: usize, b: &Box3) -> Grid {
    FOREST.with_borrow_mut(|f| {
        if f.as_ref().is_none_or(|(s, _)| *s != seed) {
            *f = Some((seed, forest(seed).0));
        }
        generate(&f.as_ref().expect("made").1, b, level as u32).into_grid()
    })
}

/** Tiles either way of a grid's centre tile: enough to reach REACH sites around the camera wherever it is within RECENTRE. */
const RADIUS: i64 = (2 * (REACH as i64 + RECENTRE) - TILE + 2 * TILE - 1) / (2 * TILE);

fn holds(centre: Option<Tile>, (i, k): Tile) -> bool {
    centre.is_some_and(|(ci, ck)| (i - ci).abs() <= RADIUS && (k - ck).abs() <= RADIUS)
}

/** The box of tiles from `from` to `to`, inclusive, at `level`. */
fn tiles_box(level: usize, from: Tile, to: Tile) -> Box3 {
    let (min_y, max_y) = heights(level as u32);
    Box3 { min: [from.0 * TILE, min_y, from.1 * TILE], max: [(to.0 + 1) * TILE, max_y, (to.1 + 1) * TILE] }
}

impl World {
    /** A world generated and indexed on `threads` threads. */
    pub fn new(seed: u64, lattices: Lattices, threads: usize) -> World {
        let (send, done) = channel();
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a thread pool");
        World {
            seed,
            lattices,
            pool: Arc::new(pool),
            grids: vec![None; GRID_LEVELS],
            centres: vec![None; GRID_LEVELS],
            pending: (0..GRID_LEVELS).map(|_| None).collect(),
            send,
            done,
            early: Vec::new(),
        }
    }

    /** Whether a grid is being made. */
    pub fn busy(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }

    /** Waits until a tile or a grid is made. */
    pub fn wait(&mut self) {
        self.early.extend(self.done.recv());
    }

    /** Starts the grids the camera (lattice units) needs, finest level first, and returns those made whole since the last call. */
    pub fn update(&mut self, camera: Vec3) -> Vec<NewGrid> {
        for level in 0..GRID_LEVELS {
            self.start(level, camera);
        }
        let mut made = Vec::new();
        let done: Vec<Done> = std::mem::take(&mut self.early).into_iter().chain(self.done.try_iter()).collect();
        for done in done {
            match done {
                Done::Part { level, tile, grid } => {
                    let Some(p) = &mut self.pending[level] else { continue };
                    if p.tiles.contains(&tile) {
                        p.received.push(grid);
                        if p.received.len() == p.tiles.len() {
                            self.compose(level);
                        }
                    }
                }
                Done::Grid { centre, grid } => {
                    self.grids[grid.level] = Some(grid.grid.clone());
                    self.centres[grid.level] = Some(centre);
                    self.pending[grid.level] = None;
                    made.push(grid);
                }
            }
        }
        made
    }

    /** Starts a grid of `level` centred on the camera's tile, unless one is being made or the grid is near enough. */
    fn start(&mut self, level: usize, camera: Vec3) {
        if self.pending[level].is_some() {
            return;
        }
        let n = (1u64 << level) as f64;
        let off = |v: f64, c: i64| (v / n - (c as f64 + 0.5) * TILE as f64).abs() > RECENTRE as f64;
        let old = self.centres[level];
        if old.is_some_and(|(i, k)| !off(camera[0], i) && !off(camera[2], k)) {
            return;
        }
        let centre = ((camera[0] / n / TILE as f64).floor() as i64, (camera[2] / n / TILE as f64).floor() as i64);
        let mut tiles = Vec::new();
        for k in centre.1 - RADIUS..=centre.1 + RADIUS {
            for i in centre.0 - RADIUS..=centre.0 + RADIUS {
                if !holds(old, (i, k)) {
                    tiles.push((i, k));
                }
            }
        }
        for &tile in &tiles {
            let (seed, send) = (self.seed, self.send.clone());
            self.pool.spawn(move || {
                let grid = generate_tile(seed, level, &tiles_box(level, tile, tile));
                // Gone if the world is.
                let _ = send.send(Done::Part { level, tile, grid });
            });
        }
        let empty = tiles.is_empty();
        self.pending[level] = Some(Pending { centre, tiles, received: Vec::new() });
        if empty {
            self.compose(level);
        }
    }

    /** Composes the grid being made of `level` from its tiles and the old grid's, with its LayerIndex, off the main thread. */
    fn compose(&mut self, level: usize) {
        let p = self.pending[level].as_mut().expect("being made");
        let (centre, received) = (p.centre, std::mem::take(&mut p.received));
        let (old, lattices, send) = (self.grids[level].clone(), self.lattices.clone(), self.send.clone());
        self.pool.spawn(move || {
            let b = tiles_box(level, (centre.0 - RADIUS, centre.1 - RADIUS), (centre.0 + RADIUS, centre.1 + RADIUS));
            let parts: Vec<&Grid> = old.as_deref().into_iter().chain(&received).collect();
            let grid = compose(&b, &parts);
            let index = LayerIndex::new(&grid, &lattices);
            let _ = send.send(Done::Grid { centre, grid: NewGrid { level, grid: Arc::new(grid), index } });
        });
    }
}
