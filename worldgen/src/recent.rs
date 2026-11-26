//! What was used last, kept: a population's things measured, a settlement's
//! layout (`Memo`); plans' columns and points' cells, small, the last in
//! each slot (`Kept`).

use std::cell::UnsafeCell;
use std::hash::Hash;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU32, Ordering, fence};
use std::sync::{Arc, Mutex};

use lattice::FastMap;

/**
 * The values of the keys used last: those put or read since it last held
 * `kept`, and those of before, forgotten when it holds as many again.
 * So what is in use is never forgotten, as all of a map cleared when full is.
 */
pub(crate) struct Recent<K, V> {
    kept: usize,
    new: FastMap<K, V>,
    old: FastMap<K, V>,
}

impl<K: Clone + Eq + Hash, V> Recent<K, V> {
    pub(crate) fn new(kept: usize) -> Recent<K, V> {
        Recent { kept, new: FastMap::default(), old: FastMap::default() }
    }

    pub(crate) fn get(&mut self, key: &K) -> Option<&mut V> {
        if !self.new.contains_key(key) {
            let value = self.old.remove(key)?;
            self.insert(key.clone(), value);
        }
        self.new.get_mut(key)
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        if self.new.len() == self.kept {
            self.old = std::mem::take(&mut self.new);
        }
        self.new.insert(key, value);
    }
}

/**
 * Large values by key, made when first asked for and shared by all who
 * ask, the ones asked for last kept (Recent): each a function of the
 * world alone, so two threads making the same at once make the same.
 */
pub struct Memo<K, V>(Mutex<Recent<K, Arc<V>>>);

impl<K: Clone + Eq + Hash, V> Memo<K, V> {
    /** None made yet: to keep `kept`, and as many asked for before them. */
    pub fn new(kept: usize) -> Memo<K, V> {
        Memo(Mutex::new(Recent::new(kept)))
    }

    /** The value of `key`: kept, or made now with `make`, unlocked meanwhile: making one may ask for others. */
    pub fn of(&self, key: &K, make: impl FnOnce() -> V) -> Arc<V> {
        if let Some(value) = self.0.lock().expect("memo").get(key) {
            return value.clone();
        }
        let value = Arc::new(make());
        self.0.lock().expect("memo").insert(key.clone(), value.clone());
        value
    }
}

/**
 * Small values by key, the last kept in each of a number of slots, for all
 * the threads that read a world together: read without waiting, as nearly
 * every read finds its key kept. A value is a function of its key alone,
 * so one not found kept, or not kept at all, is only made again. Slots are
 * zeros till written, so pages of slots never written take no memory:
 * a plan of a few large cells takes a few pages.
 */
pub struct Kept<K, V>(Box<[Slot<K, V>]>);

/**
 * A slot: a key and its value, and how many times it was written, odd
 * while it is (a sequence lock); none, 0, while it holds no key. A reader
 * copies it and takes the copy only if no one wrote meanwhile. By the
 * letter its read may race a write, which Rust does not allow even of
 * bytes dropped: as sequence locks do.
 */
struct Slot<K, V> {
    written: AtomicU32,
    kept: UnsafeCell<MaybeUninit<(K, V)>>,
}

// SAFETY: `kept` is written only by the one that made `written` odd, and what is read of it meanwhile is not used.
unsafe impl<K: Send, V: Send> Sync for Slot<K, V> {}

impl<K: Copy + PartialEq, V: Copy> Kept<K, V> {
    pub fn new(slots: usize) -> Kept<K, V> {
        // SAFETY: a slot all zeros is one never written: `written` 0, and what it keeps, any bytes, not yet a value.
        Kept(unsafe { Box::<[Slot<K, V>]>::new_zeroed_slice(slots).assume_init() })
    }

    /** The value of `key`, if kept in `slot`. */
    #[inline]
    pub fn get(&self, slot: usize, key: K) -> Option<V> {
        let slot = &self.0[slot];
        let before = slot.written.load(Ordering::Acquire);
        // SAFETY: the copy may be torn by one writing now, so it is no value yet: only bytes.
        let copy = unsafe { std::ptr::read_volatile(slot.kept.get()) };
        fence(Ordering::Acquire);
        // Never written (or wrapped round to 0, as if never), being written, or written meanwhile: no key's value.
        if before == 0 || before & 1 != 0 || slot.written.load(Ordering::Relaxed) != before {
            return None;
        }
        // SAFETY: written, and by none while it was copied, so it is what the last one to write kept, whole.
        let (k, value) = unsafe { copy.assume_init() };
        (k == key).then_some(value)
    }

    /** Keeps `key`'s `value` in `slot`, unless another thread is keeping one there now. */
    pub fn keep(&self, slot: usize, key: K, value: V) {
        let slot = &self.0[slot];
        let before = slot.written.load(Ordering::Relaxed);
        if before & 1 == 0 && slot.written.compare_exchange(before, before + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            // SAFETY: `written` is odd by this thread, so no other writes, and readers drop what they copy.
            unsafe { slot.kept.get().write(MaybeUninit::new((key, value))) };
            slot.written.store(before.wrapping_add(2), Ordering::Release);
        }
    }

    /** The value of `key`: kept in `slot`, or made now with `make` and kept. */
    #[inline]
    pub fn get_or(&self, slot: usize, key: K, make: impl FnOnce() -> V) -> V {
        self.get(slot, key).unwrap_or_else(|| {
            let value = make();
            self.keep(slot, key, value);
            value
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::Rng;

    /** Threads keeping keys in the same few slots, each a value many words long that says its key: none reads one torn, or another's. */
    #[test]
    fn a_key_read_while_others_are_kept_is_whole_or_not_found() {
        let kept = Kept::<[i64; 2], [i64; 16]>::new(4);
        let found = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for thread in 0..4 {
                let (kept, found) = (&kept, &found);
                scope.spawn(move || {
                    let mut random = Rng::new(thread);
                    for _ in 0..1 << 20 {
                        let (slot, x, z) = (random.below(4) as usize, random.below(8) as i64, random.below(8) as i64);
                        match kept.get(slot, [x, z]) {
                            Some(value) => {
                                assert_eq!(value, [x * 8 + z; 16], "key ({x}, {z})");
                                found.fetch_add(1, Ordering::Relaxed);
                            }
                            None => kept.keep(slot, [x, z], [x * 8 + z; 16]),
                        }
                    }
                });
            }
        });
        assert!(found.load(Ordering::Relaxed) > 0, "some keys found kept");
    }

    #[test]
    fn a_slot_never_written_holds_no_key() {
        // Its zeros are key [0, 0]'s bytes, and value 0's.
        let kept = Kept::<[i64; 2], u8>::new(1);
        assert_eq!(kept.get(0, [0, 0]), None);
        kept.keep(0, [0, 0], 7);
        assert_eq!(kept.get(0, [0, 0]), Some(7));
    }
}
