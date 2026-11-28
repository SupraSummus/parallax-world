//! Threads that work a step's rounds together: each round in phases, a
//! barrier between them, in each of which a thread writes only its own part
//! of what is shared, and reads what no thread writes.

use std::marker::PhantomData;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/** Threads to work rounds on: where there are threads, as many as there are cores. */
pub fn cores() -> usize {
    #[cfg(target_arch = "wasm32")]
    return 1;
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/** Thread `t`'s part, of `threads`, of `0..n`. */
pub fn part(n: usize, t: usize, threads: usize) -> Range<usize> {
    n * t / threads..n * (t + 1) / threads
}

/** `work` on `threads` threads at once, this one among them, each told which it is: what each gives back, in order. */
pub fn together<R: Send>(threads: usize, work: impl Fn(usize, &Barrier) -> R + Sync) -> Vec<R> {
    let barrier = Barrier::new(threads);
    if threads == 1 {
        return vec![work(0, &barrier)];
    }
    let (work, barrier) = (&work, &barrier);
    // A thread that panics breaks the barrier, so the others panic too, not wait for it for ever.
    let work = move |t| {
        let _breaks = Breaks(barrier);
        work(t, barrier)
    };
    std::thread::scope(|scope| {
        let others: Vec<_> = (1..threads).map(|t| scope.spawn(move || work(t))).collect();
        let first = work(0);
        std::iter::once(first).chain(others.into_iter().map(|o| o.join().expect("a thread of the crew"))).collect()
    })
}

/**
 * Where threads wait till all have come: a round's phases are microseconds,
 * so they spin, as parking would take longer; but only a little, then
 * yield, so that where there are more threads than cores, one the system
 * put aside, that all wait for, gets its core back soon.
 */
pub struct Barrier {
    threads: usize,
    come: AtomicUsize,
    round: AtomicUsize,
    broken: AtomicBool,
}

struct Breaks<'a>(&'a Barrier);

impl Drop for Breaks<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.broken.store(true, Ordering::Release);
        }
    }
}

impl Barrier {
    fn new(threads: usize) -> Barrier {
        Barrier { threads, come: AtomicUsize::new(0), round: AtomicUsize::new(0), broken: AtomicBool::new(false) }
    }

    /** Waits till every thread has come: what each wrote before is seen by all after. */
    pub fn wait(&self) {
        if self.threads == 1 {
            return;
        }
        let round = self.round.load(Ordering::Acquire);
        if self.come.fetch_add(1, Ordering::AcqRel) + 1 == self.threads {
            self.come.store(0, Ordering::Relaxed);
            self.round.fetch_add(1, Ordering::Release);
            return;
        }
        let mut spins = 0u32;
        while self.round.load(Ordering::Acquire) == round {
            assert!(!self.broken.load(Ordering::Acquire), "another thread of the crew panicked");
            if spins < 1 << 6 {
                std::hint::spin_loop();
                spins += 1;
            } else {
                std::thread::yield_now();
            }
        }
    }
}

/**
 * A slice threads share between barriers: in each phase, each element is
 * either read by any thread or written by one, never both.
 */
pub struct Shared<'a, T> {
    at: *mut T,
    len: usize,
    of: PhantomData<&'a mut [T]>,
}

// SAFETY: what it hands out is as `&[T]` and `&mut [T]` are, held to the phases by its callers.
unsafe impl<T: Send + Sync> Sync for Shared<'_, T> {}

impl<'a, T> Shared<'a, T> {
    pub fn new(slice: &'a mut [T]) -> Shared<'a, T> {
        Shared { at: slice.as_mut_ptr(), len: slice.len(), of: PhantomData }
    }

    /**
     * All of it, to read.
     *
     * SAFETY: no thread writes any of it till the next barrier.
     */
    pub unsafe fn all(&self) -> &[T] {
        // SAFETY: the slice it was made from, borrowed for 'a; nothing writes it (the caller's word).
        unsafe { std::slice::from_raw_parts(self.at, self.len) }
    }

    /**
     * `range` of it, to write.
     *
     * SAFETY: no other thread reads or writes any of `range` till the next barrier.
     */
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn part(&self, range: Range<usize>) -> &mut [T] {
        assert!(range.start <= range.end && range.end <= self.len, "{range:?} of {}", self.len);
        // SAFETY: within the slice it was made from; only this thread touches `range` (the caller's word).
        unsafe { std::slice::from_raw_parts_mut(self.at.add(range.start), range.len()) }
    }
}
