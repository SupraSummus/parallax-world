//! Bakes chunks on worker threads (WorkerBakery in worker-bakery.ts), each
//! with its own baker sharing the grids. Every render hands over the chunks
//! it misses, most wanted first, replacing the previous list, spread over
//! the workers by cell.
//! A chunk is in flight from when its worker takes it until take() hands it
//! over, and is not queued again meanwhile. Workers bake from the latest
//! tiles of each level; a chunk is only asked for once it has all the tiles
//! it reads, so no bake goes stale.

use std::collections::{HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bake::lod::CellChunk;
use bake::slicing::{GRID_LEVELS, LAYER, half_layer, spacing};
use bake::{Baker, Tiles};

use crate::compositor::{GpuCluster, GpuColumn, GpuSplat, cluster, tiles};

/** A chunk to bake: a cell of a face, reading tiles [i0, i1] × [k0, k1] of its grid level. */
#[derive(Clone, Copy)]
pub struct Request {
    pub face: usize,
    pub cell: CellChunk,
    pub tiles: [i32; 4],
    /** The size of the blocks its see-through splats are clustered by (compositor::cluster). */
    pub block: f64,
    /** Whether its voxels are lit, by the baker's light (Baker's `light`), or shaded by the way their faces face. */
    pub lit: bool,
}

/**
 * A baked chunk, as the GPU takes it, and where it may be drawn: its
 * relief's columns (compositor::tiles) and its see-through splats
 * (compositor::cluster), and their clusters, the tiles of columns first,
 * then the see-through ones from `see_through`.
 */
pub struct Baked {
    pub origin: [f64; 2],
    pub extent: [f64; 2],
    pub columns: Vec<GpuColumn>,
    pub splats: Vec<GpuSplat>,
    pub clusters: Vec<GpuCluster>,
    pub see_through: usize,
}

impl Baked {
    /** Whether it draws nothing. */
    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }
}

/** What a request bakes: face and cell. */
type Key = (usize, CellChunk);

impl Request {
    fn key(&self) -> Key {
        (self.face, self.cell)
    }

    /** The worker of the request's chunk. */
    fn worker(&self, workers: usize) -> usize {
        let mut h = DefaultHasher::new();
        self.key().hash(&mut h);
        h.finish() as usize % workers
    }
}

/** A worker's requests to take, and those it has taken that take() has not handed over yet. */
#[derive(Default)]
struct Work {
    queued: VecDeque<Request>,
    in_flight: HashSet<Key>,
}

type Queue = (Mutex<Work>, Condvar);

/** By grid level: its latest tiles, which workers take before baking from them. */
type Grids = Arc<Mutex<Vec<Arc<Tiles>>>>;

pub struct Bakery {
    queues: Vec<Arc<Queue>>,
    grids: Grids,
    baked: Receiver<(Request, Baked)>,
    /** Received by wait(), not taken yet. */
    early: Vec<(Request, Baked)>,
}

impl Bakery {
    /** Starts `threads` workers, each baking with a baker from `baker`, its light kept aside for requests not lit. */
    pub fn new(threads: usize, baker: impl Fn() -> Baker + Send + Sync + 'static) -> Bakery {
        let (send, baked) = channel();
        let baker = Arc::new(baker);
        let grids: Grids = Arc::new(Mutex::new((0..GRID_LEVELS).map(|_| Arc::new(Tiles::new(0, []))).collect()));
        let queues = (0..threads)
            .map(|_| {
                let queue: Arc<Queue> = Arc::default();
                let (queue2, send, baker, grids) = (queue.clone(), send.clone(), baker.clone(), grids.clone());
                std::thread::spawn(move || work(&queue2, &send, &grids, baker()));
                queue
            })
            .collect();
        Bakery { queues, grids, baked, early: Vec::new() }
    }

    /** The tiles of `level` from now on. */
    pub fn set_tiles(&self, level: usize, tiles: Arc<Tiles>) {
        self.grids.lock().expect("grids")[level] = tiles;
    }

    /** The chunks to bake, most wanted first; replaces the previous list. */
    pub fn want(&self, requests: &[Request]) {
        let mut lists = vec![VecDeque::new(); self.queues.len()];
        for r in requests {
            lists[r.worker(self.queues.len())].push_back(*r);
        }
        for (queue, mut list) in self.queues.iter().zip(lists) {
            let mut work = queue.0.lock().expect("queue");
            list.retain(|r| !work.in_flight.contains(&r.key()));
            work.queued = list;
            queue.1.notify_one();
        }
    }

    /** The chunks baked since the last call. */
    pub fn take(&mut self) -> Vec<(Request, Baked)> {
        let mut baked = std::mem::take(&mut self.early);
        baked.extend(self.baked.try_iter());
        for (r, _) in &baked {
            self.queues[r.worker(self.queues.len())].0.lock().expect("queue").in_flight.remove(&r.key());
        }
        baked
    }

    /** Waits until a chunk is baked. */
    pub fn wait(&mut self) {
        self.early.extend(self.baked.recv());
    }
}

fn work(queue: &Queue, send: &Sender<(Request, Baked)>, grids: &Grids, mut baker: Baker) {
    // The baker's light while it bakes unlit.
    let mut aside = None;
    // The tiles of each level the baker has: none yet.
    let mut has: Vec<Arc<Tiles>> = (0..GRID_LEVELS).map(|_| Arc::new(Tiles::new(0, []))).collect();
    loop {
        let request = {
            let mut work = queue.0.lock().expect("queue");
            loop {
                if let Some(r) = work.queued.pop_front() {
                    work.in_flight.insert(r.key());
                    break r;
                }
                work = queue.1.wait(work).expect("queue");
            }
        };
        let r = &request;
        let level = r.cell.atoms as usize;
        let latest = grids.lock().expect("grids")[level].clone();
        if !Arc::ptr_eq(&latest, &has[level]) {
            baker.share_tiles(level, latest.clone());
            has[level] = latest;
        }
        // Dropped meanwhile, the camera gone far: not wanted any more.
        if !has[level].has_all(r.tiles) {
            queue.0.lock().expect("queue").in_flight.remove(&r.key());
            continue;
        }
        if r.lit != baker.light.is_some() {
            std::mem::swap(&mut baker.light, &mut aside);
        }
        let b = baker.bake_cell(r.face, &r.cell);
        // Its column (0, 0): the least (a, b) of its cell's bounds, relative to its origin.
        let unit = spacing(r.cell.atoms) as f64;
        let [a, b0, _] = r.cell.bounds().0;
        let base = [a as f64 * LAYER * unit - b.origin_x, b0 as f64 * unit - b.origin_y];
        let half_layer = half_layer(r.cell.atoms);
        let (columns, mut clusters) = tiles(&baker.columns, base, unit, half_layer);
        let see_through = clusters.len();
        let (splats, veils) = cluster(&baker.numbers, half_layer, r.block);
        clusters.extend(veils);
        let baked =
            Baked { origin: [b.origin_x, b.origin_y], extent: [b.extent_width, b.extent_height], columns, splats, clusters, see_through };
        if send.send((request, baked)).is_err() {
            return;
        }
    }
}
