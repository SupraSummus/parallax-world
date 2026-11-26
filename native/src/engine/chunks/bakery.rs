//! Bakes chunks on worker threads, each
//! with its own baker sharing the grids. Every render hands over the chunks
//! it misses, most wanted first, replacing the previous list, spread over
//! the workers by cell.
//! A chunk is in flight from when its worker takes it until take() hands it
//! over, and is not queued again meanwhile. Workers bake from the latest
//! tiles of each level; a chunk is only asked for once it has all the tiles
//! it reads, so no bake goes stale. Dropped, it stops its workers.

use std::collections::{HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bake::lod::CellChunk;
use bake::{Baker, half_layer};
use lattice::GRID_LEVELS;
use lattice::{TileId, Tiles};

use super::compositor::{Flat, GpuCluster, GpuColumn, flatten, veils};

/** A chunk to bake: a cell of a face, reading the tiles of its grid level from the first to the last, each way. */
#[derive(Clone, Copy)]
pub struct Request {
    pub face: usize,
    pub cell: CellChunk,
    pub tiles: [TileId; 2],
    /** Whether its voxels are lit, by the baker's light (Baker's `light`), or shaded by the way their faces face. */
    pub lit: bool,
}

/**
 * A baked chunk, as the GPU takes it, and where it is drawn from: its
 * picture (compositor::flatten), then its medium's pictures
 * (compositor::veils), and their clusters: its picture's runs first, then
 * the medium's from `veils`.
 */
pub struct Baked {
    pub origin: [f64; 2],
    pub columns: Vec<GpuColumn>,
    pub clusters: Vec<GpuCluster>,
    pub veils: usize,
    /** Its picture's plane (compositor::flatten). */
    pub flat: Flat,
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

/** A worker's requests to take, those it has taken that take() has not handed over yet, and whether it is to stop. */
#[derive(Default)]
struct Work {
    queued: VecDeque<Request>,
    in_flight: HashSet<Key>,
    stop: bool,
}

type Queue = (Mutex<Work>, Condvar);

/** By grid level: its latest tiles, which workers take before baking from them. */
type Grids = Arc<Mutex<Vec<Arc<Tiles>>>>;

pub struct Bakery {
    queues: Vec<Arc<Queue>>,
    grids: Grids,
    baked: Receiver<(Request, Baked)>,
}

impl Bakery {
    /**
     * Starts `threads` workers, each baking with a baker from `baker`, its
     * light kept aside for requests not lit, and calling `delivered` after
     * each chunk it hands over.
     */
    pub fn new(threads: usize, delivered: impl Fn() + Send + Sync + 'static, baker: impl Fn() -> Baker + Send + Sync + 'static) -> Bakery {
        let (send, baked) = channel();
        let (baker, delivered) = (Arc::new(baker), Arc::new(delivered));
        let grids: Grids = Arc::new(Mutex::new((0..GRID_LEVELS).map(|_| Arc::new(Tiles::new(0, []))).collect()));
        let queues = (0..threads)
            .map(|_| {
                let queue: Arc<Queue> = Arc::default();
                let (queue2, send, baker, grids, delivered) =
                    (queue.clone(), send.clone(), baker.clone(), grids.clone(), delivered.clone());
                std::thread::spawn(move || work(&queue2, &send, &grids, baker(), &*delivered));
                queue
            })
            .collect();
        Bakery { queues, grids, baked }
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
        let baked: Vec<(Request, Baked)> = self.baked.try_iter().collect();
        for (r, _) in &baked {
            self.queues[r.worker(self.queues.len())].0.lock().expect("queue").in_flight.remove(&r.key());
        }
        baked
    }
}

impl Drop for Bakery {
    fn drop(&mut self) {
        for queue in &self.queues {
            queue.0.lock().expect("queue").stop = true;
            queue.1.notify_one();
        }
    }
}

fn work(queue: &Queue, send: &Sender<(Request, Baked)>, grids: &Grids, mut baker: Baker, delivered: &dyn Fn()) {
    crate::world::behind_drawing();
    // The baker's light while it bakes unlit.
    let mut aside = None;
    // The tiles of each level the baker has: none yet.
    let mut has: Vec<Arc<Tiles>> = (0..GRID_LEVELS).map(|_| Arc::new(Tiles::new(0, []))).collect();
    loop {
        let request = {
            let mut work = queue.0.lock().expect("queue");
            loop {
                if work.stop {
                    return;
                }
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
        let origin = baker.bake_cell(r.face, &r.cell);
        let face = &bake::face::faces()[r.face];
        // Its columns from its column (0, 0), at its origin.
        let unit = f64::from(1 << r.cell.atoms);
        let half_layer = half_layer(face, r.cell.atoms);
        let (mut columns, mut clusters, flat) = flatten(face, &baker.columns, unit, half_layer);
        let veils_at = clusters.len();
        let slab = bake::slab_layers(r.cell.spacing());
        let (medium, medium_clusters) = veils(face, &baker.medium, unit, half_layer, slab, columns.len());
        columns.extend(medium);
        clusters.extend(medium_clusters);
        let baked = Baked { origin, columns, clusters, veils: veils_at, flat };
        if send.send((request, baked)).is_err() {
            return;
        }
        delivered();
    }
}
