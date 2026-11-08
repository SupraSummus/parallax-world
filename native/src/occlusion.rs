//! Which cells in view nearer layers cover (bake's Coverage), on a thread
//! of its own, from what renders saw: covered cells are neither baked nor
//! have their tiles made. The frame draws every chunk in view regardless;
//! the compositor culls what is hidden on the GPU.
//! A render uses the latest answer, which may be for an earlier view;
//! a cell it has no answer for waits. See docs/architecture.md, section 4.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use bake::Coverage;

use crate::bakery::Mask;
use crate::renderer::Chunk;
use crate::view::{ScreenBox, Vec3};

/** A cell of a slice: face, slice (level, layer, cutoff as bits), cell (i, j). */
pub type CellKey = (usize, i32, i32, u64, i32, i32);

/** A cell in view as a render saw it. */
pub struct Seen {
    pub key: CellKey,
    /** Where it may draw on screen. */
    pub screen: ScreenBox,
    /** Its rectangle on its layer's plane, relative to the camera: left, bottom, width, height. */
    pub rect: [f64; 4],
    /** Its chunk, if baked and on screen, and where it is: its top-left corner. */
    pub shown: Option<(Arc<Chunk>, f64, f64)>,
    /** Whether it asks for something: a chunk baked, or tiles made. */
    pub asks: bool,
}

/** A layer's cells in view, `distance` ahead along its face's axis. */
pub struct Layer {
    pub distance: f64,
    pub cells: Vec<Seen>,
}

/** A face in view: the camera's right, up and forward in its coordinates, and its layers near to far. */
pub struct Face {
    pub face: usize,
    pub basis: [Vec3; 3],
    pub layers: Vec<Layer>,
}

/** What a render saw: the viewport in CSS pixels and focal length, and its faces. */
pub struct Seeing {
    pub width: f64,
    pub height: f64,
    pub focal: f64,
    pub faces: Vec<Face>,
}

impl Seeing {
    /** The same for what would get the same answer. */
    fn signature(&self) -> u64 {
        let mut h = DefaultHasher::new();
        [self.width, self.height, self.focal].map(f64::to_bits).hash(&mut h);
        for face in &self.faces {
            (face.face, face.basis.map(|axis| axis.map(f64::to_bits))).hash(&mut h);
            for layer in &face.layers {
                layer.distance.to_bits().hash(&mut h);
                for seen in &layer.cells {
                    (seen.key, seen.asks, seen.shown.as_ref().map(|(chunk, x, y)| (chunk.id, x.to_bits(), y.to_bits()))).hash(&mut h);
                }
            }
        }
        h.finish()
    }
}

/** A pass's answer: for each cell that asked, whether it is covered. */
#[derive(Default)]
pub struct Answer {
    /** The pass it answers, counting from 1; 0 for none yet. */
    pub pass: u64,
    pub covered: HashMap<CellKey, bool>,
    pub seconds: f64,
}

#[derive(Default)]
struct State {
    /** What to pass over next, and its number. */
    next: Option<(u64, Seeing)>,
    asked: u64,
    /** Of what was asked last. */
    signature: u64,
    answer: Arc<Answer>,
    stop: bool,
}

type Shared = (Mutex<State>, Condvar);

/** The thread and what it shares with the renderer. */
pub struct Occlusion {
    shared: Arc<Shared>,
}

impl Occlusion {
    pub fn new() -> Occlusion {
        let shared: Arc<Shared> = Arc::default();
        let theirs = shared.clone();
        std::thread::spawn(move || work(&theirs));
        Occlusion { shared }
    }

    /** Passes over `seeing` next, instead of what is waiting, unless it is what was asked last. */
    pub fn ask(&self, seeing: Seeing) {
        let signature = seeing.signature();
        let mut state = self.shared.0.lock().expect("occlusion");
        if state.asked > 0 && signature == state.signature {
            return;
        }
        state.asked += 1;
        state.signature = signature;
        state.next = Some((state.asked, seeing));
        self.shared.1.notify_all();
    }

    /** The latest answer, and whether one for what was asked last is yet to come. */
    pub fn answer(&self) -> (Arc<Answer>, bool) {
        let state = self.shared.0.lock().expect("occlusion");
        (state.answer.clone(), state.answer.pass < state.asked)
    }

    /** Waits until what was asked last is answered. */
    pub fn wait(&self) {
        let mut state = self.shared.0.lock().expect("occlusion");
        while state.answer.pass < state.asked {
            state = self.shared.1.wait(state).expect("occlusion");
        }
    }
}

impl Drop for Occlusion {
    fn drop(&mut self) {
        self.shared.0.lock().expect("occlusion").stop = true;
        self.shared.1.notify_all();
    }
}

fn work(shared: &Shared) {
    let mut coverage = Coverage::default();
    loop {
        let (pass, seeing) = {
            let mut state = shared.0.lock().expect("occlusion");
            loop {
                if state.stop {
                    return;
                }
                if let Some(next) = state.next.take() {
                    break next;
                }
                state = shared.1.wait(state).expect("occlusion");
            }
        };
        let answer = Arc::new(answer(&mut coverage, pass, &seeing));
        shared.0.lock().expect("occlusion").answer = answer;
        shared.1.notify_all();
    }
}

/** Hands a chunk's mask to the coverage unless it has it. */
fn known(coverage: &mut Coverage, chunk: &Chunk) -> u32 {
    if !coverage.use_chunk(chunk.id) {
        let Mask { width, height, scale, ref blocks } = chunk.baked.mask;
        coverage.add(chunk.id, width, height, scale, blocks, &[]);
    }
    chunk.id
}

/**
 * Covers the screen face by face, layer by layer near to far (drawSlice in
 * renderer.ts): a cell is covered if nearer layers cover its box or its
 * chunk; a layer's chunks not covered then cover what is behind it.
 */
fn answer(coverage: &mut Coverage, pass: u64, seeing: &Seeing) -> Answer {
    let start = Instant::now();
    let mut answer = Answer { pass, ..Answer::default() };
    let mut covering = Vec::new();
    coverage.set_view(seeing.width, seeing.height, seeing.focal);
    for face in &seeing.faces {
        // Layers past the farthest that asks cover nothing that is asked about.
        let Some(last) = face.layers.iter().rposition(|layer| layer.cells.iter().any(|seen| seen.asks)) else { continue };
        let [right, up, forward] = face.basis;
        coverage.set_face(face.face, right, up, forward);
        for layer in &face.layers[..=last] {
            for seen in &layer.cells {
                let ScreenBox { min_x, max_x, min_y, max_y } = seen.screen;
                let mut covered = coverage.hides(min_x, max_x, min_y, max_y);
                if !covered && let Some((chunk, x, y)) = &seen.shown {
                    let handle = known(coverage, chunk);
                    covered = coverage.hides_chunk(handle, *x, *y, layer.distance);
                    if !covered {
                        covering.push((handle, *x, *y, seen.rect));
                    }
                }
                if seen.asks {
                    answer.covered.insert(seen.key, covered);
                }
            }
            for (handle, x, y, [left, bottom, width, height]) in covering.drain(..) {
                coverage.cover(handle, x, y, left, bottom, width, height, layer.distance);
            }
            coverage.end_layer(layer.distance);
        }
    }
    answer.seconds = start.elapsed().as_secs_f64();
    answer
}
