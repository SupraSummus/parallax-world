//! Bakes chunks on worker threads (WorkerBakery in worker-bakery.ts), each
//! with its own baker sharing the grids. Every render hands over the chunks
//! it misses, most wanted first, replacing the previous list; a cell always
//! goes to the same worker, which keeps its surface for other eyes.

use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bake::{Baker, Slice};

/** A chunk to bake: cell (i, j) of a slice of a face, from `eye`, grown if `fill`. */
#[derive(Clone, Copy)]
pub struct Request {
    pub face: usize,
    pub slice: Slice,
    pub i: i32,
    pub j: i32,
    pub eye: [i32; 3],
    pub fill: bool,
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

type Queue = (Mutex<VecDeque<Request>>, Condvar);

pub struct Bakery {
    queues: Vec<Arc<Queue>>,
    baked: Receiver<(Request, Baked)>,
    /** Received by wait(), not taken yet. */
    early: Vec<(Request, Baked)>,
}

impl Bakery {
    /** Starts `threads` workers, each baking with a baker from `baker`. */
    pub fn new(threads: usize, baker: impl Fn() -> Baker + Send + Sync + 'static) -> Bakery {
        let (send, baked) = channel();
        let baker = Arc::new(baker);
        let queues = (0..threads)
            .map(|_| {
                let queue: Arc<Queue> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
                let (queue2, send, baker) = (queue.clone(), send.clone(), baker.clone());
                std::thread::spawn(move || work(&queue2, &send, baker()));
                queue
            })
            .collect();
        Bakery { queues, baked, early: Vec::new() }
    }

    /** The chunks to bake, most wanted first; replaces the previous list. */
    pub fn want(&self, requests: &[Request]) {
        let mut lists = vec![VecDeque::new(); self.queues.len()];
        for r in requests {
            let mut h = DefaultHasher::new();
            (r.face, r.slice.level, r.slice.layer, r.slice.cutoff.to_bits(), r.i, r.j).hash(&mut h);
            lists[h.finish() as usize % self.queues.len()].push_back(*r);
        }
        for (queue, list) in self.queues.iter().zip(lists) {
            *queue.0.lock().expect("queue") = list;
            queue.1.notify_one();
        }
    }

    /** The chunks baked since the last call. */
    pub fn take(&mut self) -> Vec<(Request, Baked)> {
        let mut baked = std::mem::take(&mut self.early);
        baked.extend(self.baked.try_iter());
        baked
    }

    /** Waits until a chunk is baked. */
    pub fn wait(&mut self) {
        self.early.extend(self.baked.recv());
    }
}

fn work(queue: &Queue, send: &Sender<(Request, Baked)>, mut baker: Baker) {
    loop {
        let request = {
            let mut list = queue.0.lock().expect("queue");
            loop {
                if let Some(r) = list.pop_front() {
                    break r;
                }
                list = queue.1.wait(list).expect("queue");
            }
        };
        let r = &request;
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
