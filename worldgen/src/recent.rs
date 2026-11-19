//! What was used last, kept: a Light's sites, a population's things measured.

use fcc::FastMap;

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

impl<K: Clone + Eq + std::hash::Hash, V> Recent<K, V> {
    pub(crate) fn new(kept: usize) -> Recent<K, V> {
        Recent { kept, new: FastMap::default(), old: FastMap::default() }
    }

    pub(crate) fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.new.retain(|key, _| keep(key));
        self.old.retain(|key, _| keep(key));
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
