/// A monoid.
pub trait Merge: Default {
    fn merge(&mut self, other: Self);
}

/// An ordered map of mergeable values: a sorted prefix without duplicate
/// keys, then the values of new keys in any order (consecutive ones
/// merged), so that a key may have several partial values until sorting,
/// which merges them in the order they were created. Dense for small maps,
/// and amortized O(log n) per access for large ones.
#[derive(Debug, Clone)]
pub struct MergeMap<K, V> {
    entries: Vec<(K, V)>,
    sorted: usize,
}

impl<K, V> Default for MergeMap<K, V> {
    fn default() -> MergeMap<K, V> {
        MergeMap {
            entries: Vec::new(),
            sorted: 0,
        }
    }
}

impl<K: Ord, V: Merge> MergeMap<K, V> {
    const MIN_TAIL: usize = 8;

    pub fn new() -> MergeMap<K, V> {
        MergeMap::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The last value of the key to add to, which is now or later merged with
    /// other values of the key.
    pub fn get_mut(&mut self, key: K) -> &mut V {
        if self.entries.len() - self.sorted > self.sorted.max(Self::MIN_TAIL) {
            self.sort();
        }
        if self.entries.last().is_some_and(|(last, _)| last == &key) {
            return &mut self.entries.last_mut().expect("last").1;
        }
        match self.entries[..self.sorted].binary_search_by(|(other, _)| other.cmp(&key)) {
            Ok(i) => &mut self.entries[i].1,
            Err(_) => {
                self.entries.push((key, V::default()));
                &mut self.entries.last_mut().expect("value of key").1
            }
        }
    }

    /// The entries in key order, one per key.
    pub fn sorted_mut(&mut self) -> &mut [(K, V)] {
        self.sort();
        &mut self.entries
    }

    /// The entries in key order, one per key.
    pub fn into_sorted(mut self) -> Vec<(K, V)> {
        self.sort();
        self.entries
    }

    fn sort(&mut self) {
        if self.sorted == self.entries.len() {
            return;
        }
        self.entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        self.entries.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1.merge(std::mem::take(&mut later.1));
            }
            same
        });
        self.sorted = self.entries.len();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use arbtest::arbtest;

    use super::*;

    /// Concatenation: associative, but not commutative.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Log(Vec<u32>);

    impl Merge for Log {
        fn merge(&mut self, other: Log) {
            self.0.extend(other.0);
        }
    }

    #[test]
    fn test_merge_map_keeps_order_of_additions() {
        arbtest(|u| {
            let mut map: MergeMap<u8, Log> = MergeMap::new();
            let mut model: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
            let max_key = u8::MAX >> u.int_in_range(0..=7)?;
            for i in 0..u.int_in_range(0..=500)? {
                let key = u.int_in_range(0..=max_key)?;
                map.get_mut(key).0.push(i);
                model.entry(key).or_default().push(i);
                if u.ratio(1, 100)? {
                    map.sorted_mut();
                }
            }
            let expected: Vec<(u8, Log)> = model.into_iter().map(|(k, v)| (k, Log(v))).collect();
            assert_eq!(map.into_sorted(), expected);
            Ok(())
        });
    }
}
