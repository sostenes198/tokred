//! A map with a size cap that forgets the least recently used entries first.

use std::collections::HashMap;
use std::hash::Hash;

/// Bounded map. When full, an insert drops the least recently used eighth of the entries.
#[derive(Debug, Clone)]
pub(crate) struct Lru<K, V> {
    map: HashMap<K, (V, u64)>,
    tick: u64,
    cap: usize,
}

impl<K: Eq + Hash + Clone, V> Lru<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            cap: cap.max(1),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// The entry, marked as just used.
    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|(value, used)| {
            *used = tick;
            value
        })
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        if self.map.len() >= self.cap && !self.map.contains_key(&key) {
            self.evict();
        }
        self.tick += 1;
        self.map.insert(key, (value, self.tick));
    }

    fn evict(&mut self) {
        let mut ticks: Vec<u64> = self.map.values().map(|(_, used)| *used).collect();
        ticks.sort_unstable();
        let drop = (self.cap / 8).max(1);
        let cutoff = ticks.get(drop - 1).copied().unwrap_or_default();
        self.map.retain(|_, (_, used)| *used > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_used_entries_go_first() {
        let mut lru = Lru::new(4);
        for key in 0..4 {
            lru.insert(key, key * 10);
        }
        assert_eq!(
            lru.get_mut(&0).copied(),
            Some(0),
            "0 is now the most recent"
        );
        lru.insert(4, 40);
        assert_eq!(lru.len(), 4);
        assert!(lru.get_mut(&1).is_none(), "1 was the least recently used");
        assert!(lru.get_mut(&0).is_some());
        // Replacing an existing key never evicts.
        lru.insert(0, 1);
        assert_eq!(lru.len(), 4);
        assert_eq!(lru.get_mut(&0).copied(), Some(1));
        assert!(lru.get_mut(&9).is_none());
    }

    #[test]
    fn a_zero_cap_holds_one_entry() {
        let mut lru = Lru::new(0);
        lru.insert("a", 1);
        lru.insert("b", 2);
        assert_eq!(lru.len(), 1);
        assert!(lru.get_mut(&"b").is_some());
    }
}
