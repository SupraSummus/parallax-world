//! Threads the thread that draws never waits for, natively and in a
//! browser alike, as a browser's main thread may not wait: it posts them
//! work (Post), which wakes them, and they take it without blocking it;
//! they park when they have none. It takes no lock they take, and reads
//! what they send back without waiting.

use std::sync::atomic::{Ordering, fence};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::thread::Thread;

/** A channel to threads that park when idle: posting wakes them, and dropped, they find it gone, and end. */
pub struct Post<T> {
    // Let go of before the threads are woken (Sleepers' Drop).
    to: Sender<T>,
    threads: Sleepers,
}

impl<T> Post<T> {
    /** A thread for each of what `works` makes of the receiving end. */
    pub fn spawn<F: FnOnce() + Send + 'static>(works: impl FnOnce(Receiver<T>) -> Vec<F>) -> Post<T> {
        let (to, from) = channel();
        Post { to, threads: Sleepers::spawn(works(from)) }
    }

    pub fn send(&self, message: T) {
        self.to.send(message).expect("the threads are gone");
        self.threads.wake();
    }
}

/** Threads to wake: each says who it is before it first looks for work. */
struct Sleepers(Arc<[OnceLock<Thread>]>);

impl Sleepers {
    fn spawn<F: FnOnce() + Send + 'static>(works: Vec<F>) -> Sleepers {
        let threads: Arc<[OnceLock<Thread>]> = works.iter().map(|_| OnceLock::new()).collect();
        for (i, work) in works.into_iter().enumerate() {
            let threads = threads.clone();
            spawn(move || {
                threads[i].set(std::thread::current()).expect("a thread once");
                // Named before it looks, as wake sends before it reads names: one sees the other.
                fence(Ordering::SeqCst);
                work()
            });
        }
        Sleepers(threads)
    }

    fn wake(&self) {
        fence(Ordering::SeqCst);
        for thread in self.0.iter().filter_map(OnceLock::get) {
            thread.unpark();
        }
    }
}

impl Drop for Sleepers {
    fn drop(&mut self) {
        self.wake();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

/** `f` on a Web Worker of its own, on this module and its memory (index.html's `spawnThread`, web/worker.js). */
#[cfg(target_arch = "wasm32")]
fn spawn(f: impl FnOnce() + Send + 'static) {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = globalThis, js_name = spawnThread)]
        fn spawn_thread(module: JsValue, memory: JsValue, work: u32);
    }
    let work: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(f));
    spawn_thread(wasm_bindgen::module(), wasm_bindgen::memory(), Box::into_raw(work) as u32);
}

/** What a Web Worker runs: `spawn`'s work. */
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run_thread(work: u32) {
    // SAFETY: `work` is what spawn let go of, handed here once.
    let work = unsafe { Box::from_raw(work as *mut Box<dyn FnOnce() + Send>) };
    work()
}

/**
 * Cores to work on. A browser puts no thread behind another
 * (world.rs's behind_drawing), so one is left to the thread that draws.
 */
pub fn cores() -> usize {
    #[cfg(target_arch = "wasm32")]
    return (web_sys::window().map_or(1.0, |w| w.navigator().hardware_concurrency()) as usize).saturating_sub(1).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::available_parallelism().map_or(1, |n| n.get())
}
