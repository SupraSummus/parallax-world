//! What was used last, kept: a population's things measured, a settlement's layout.

use std::hash::Hash;
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
