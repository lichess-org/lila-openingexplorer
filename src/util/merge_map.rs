use std::mem;

/// A monoid.
pub trait Merge: Default {
    fn merge(&mut self, other: Self);
}

/// Dense representation of an ordered map of mergeable values: a sorted prefix
/// without duplicate keys, then the values of new keys in any order
/// (consecutive ones merged).
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

    /// The last value of the key (or a default created one), which is now or
    /// later merged with other values of the key.
    pub fn get_mut(&mut self, key: K) -> &mut V {
        if self.entries.len() - self.sorted > self.sorted.max(Self::MIN_TAIL) {
            self.sort();
        }
        if self.entries.last().is_some_and(|(last, _)| last == &key) {
            return &mut self.entries.last_mut().expect("last").1;
        }
        let sorted = &self.entries[..self.sorted];
        if sorted.last().is_some_and(|(max, _)| *max >= key)
            && let Ok(i) = sorted.binary_search_by(|(other, _)| other.cmp(&key))
        {
            return &mut self.entries[i].1;
        }
        &mut self.entries.push_mut((key, V::default())).1
    }

    /// The entries in key order, one per key.
    pub fn sorted(&mut self) -> impl Iterator<Item = (&K, &V)> + Clone {
        self.sort();
        self.entries.iter().map(|(key, value)| (key, value))
    }

    /// The entries in key order, one per key.
    pub fn sorted_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.sort();
        self.entries.iter_mut().map(|(key, value)| (&*key, value))
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
                kept.1.merge(mem::take(&mut later.1));
            }
            same
        });
        self.sorted = self.entries.len();
    }
}

impl<K: Ord, V: Merge> Extend<(K, V)> for MergeMap<K, V> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, values: I) {
        self.entries.extend(values);
        self.sort();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use arbitrary::Arbitrary;
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_merge_map_keeps_order_of_additions() {
        #[derive(Arbitrary, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
        enum Key {
            One,
            Two,
            Three,
            Four,
            Five,
            Six,
            Seven,
            Eight,
        }

        #[derive(Arbitrary, Clone, Debug, Default, PartialEq, Eq)]
        struct Log(Vec<u8>);

        impl Merge for Log {
            fn merge(&mut self, other: Log) {
                self.0.extend(other.0);
            }
        }

        #[derive(Arbitrary)]
        enum Op {
            GetMutPush { key: Key, value: u8 },
            Extend(Vec<(Key, Log)>),
            Sort,
        }

        arbtest(|u| {
            let mut map: MergeMap<Key, Log> = MergeMap::new();
            let mut model: BTreeMap<Key, Log> = BTreeMap::new();
            for op in u.arbitrary_iter::<Op>()? {
                match op? {
                    Op::GetMutPush { key, value } => {
                        map.get_mut(key.clone()).0.push(value);
                        model.entry(key).or_default().0.push(value);
                    }
                    Op::Extend(tail) => {
                        map.extend(tail.clone());
                        for (key, value) in tail {
                            model.entry(key).or_default().merge(value);
                        }
                    }
                    Op::Sort => {
                        let _ = map.sorted();
                    }
                }
                assert_eq!(
                    map.clone().into_sorted(),
                    model.clone().into_iter().collect::<Vec<_>>()
                );
            }
            Ok(())
        });
    }
}
