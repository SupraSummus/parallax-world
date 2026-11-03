//! Bakes chunks on worker threads (WorkerBakery in worker-bakery.ts), each
//! with its own baker sharing the grids. Every render hands over the chunks
//! it misses, most wanted first, replacing the previous list; a cell always
//! goes to the same worker, which keeps its surface for other eyes.
//! A chunk is in flight from when its worker takes it until take() hands it
//! over, and is not queued again meanwhile.

use std::collections::{HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bake::{Baker, Grid, Slice};

use crate::slicing::{GRID_LEVELS, atom_level};

/** A chunk to bake: cell (i, j) of a slice of a face, from `eye`, grown if `fill`, from version `grid` of its level's grid. */
#[derive(Clone, Copy)]
pub struct Request {
    pub face: usize,
    pub slice: Slice,
    pub i: i32,
    pub j: i32,
    pub eye: [i32; 3],
    pub fill: bool,
    pub grid: u32,
}

/** A baked chunk (ChunkSplats in chunk-baker.ts): its splats as SPLAT numbers each, painted then grown, and its mask's blocks. */
pub struct Baked {
    pub origin: [f64; 2],
    pub extent: [f64; 2],
    pub painted: usize,
    pub numbers: Vec<f32>,
    pub mask: Mask,
}

/** Which blocks of what a chunk draws are drawn all or partly (see fill.rs), for Coverage. */
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub scale: f64,
    pub blocks: Vec<u8>,
}

/** What a request bakes: face, slice, cell, eye, fill, grid version. */
type Key = (usize, i32, i32, u64, i32, i32, [i32; 3], bool, u32);

impl Request {
    fn key(&self) -> Key {
        (self.face, self.slice.level, self.slice.layer, self.slice.cutoff.to_bits(), self.i, self.j, self.eye, self.fill, self.grid)
    }

    /** The worker of the request's cell. */
    fn worker(&self, workers: usize) -> usize {
        let mut h = DefaultHasher::new();
        (self.face, self.slice.level, self.slice.layer, self.slice.cutoff.to_bits(), self.i, self.j).hash(&mut h);
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

/** By grid level: its latest grid and version, which workers take before baking from it. */
type Grids = Arc<Mutex<Vec<Option<(u32, Arc<Grid>)>>>>;

pub struct Bakery {
    queues: Vec<Arc<Queue>>,
    grids: Grids,
    baked: Receiver<(Request, Baked)>,
    /** Received by wait(), not taken yet. */
    early: Vec<(Request, Baked)>,
}

impl Bakery {
    /** Starts `threads` workers, each baking with a baker from `baker`. */
    pub fn new(threads: usize, baker: impl Fn() -> Baker + Send + Sync + 'static) -> Bakery {
        let (send, baked) = channel();
        let baker = Arc::new(baker);
        let grids: Grids = Arc::new(Mutex::new(vec![None; GRID_LEVELS]));
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

    /** Version `version` of the grid of `level`, for chunks requested from it. */
    pub fn set_grid(&self, level: usize, grid: Arc<Grid>, version: u32) {
        self.grids.lock().expect("grids")[level] = Some((version, grid));
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
    // The version of each level's grid the baker has.
    let mut versions = [0; GRID_LEVELS];
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
        let level = atom_level(r.slice.level) as usize;
        if versions[level] != r.grid {
            let grid = grids.lock().expect("grids")[level].clone();
            if let Some((version, grid)) = grid {
                baker.share_grid(level, grid);
                versions[level] = version;
            }
            // A newer grid came meanwhile: this chunk is no longer wanted.
            if versions[level] != r.grid {
                queue.0.lock().expect("queue").in_flight.remove(&r.key());
                continue;
            }
        }
        let b = baker.bake(r.face, &r.slice, r.i, r.j, r.eye, r.fill);
        let baked = Baked {
            origin: [b.origin_x, b.origin_y],
            extent: [b.extent_width, b.extent_height],
            painted: b.painted,
            numbers: baker.numbers.clone(),
            mask: Mask { width: b.width, height: b.height, scale: b.scale, blocks: baker.blocks.clone() },
        };
        if send.send((request, baked)).is_err() {
            return;
        }
    }
}
