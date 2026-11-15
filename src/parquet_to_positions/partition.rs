use std::{sync::Mutex, thread};

use crate::{
    intermediate::IntermediateEntry, packed::ColdKey,
    parquet_to_positions::scratch_codec::ScratchCodec, scatter::ScratchFile,
};

/// Sub-buckets of a memory partition, by the hash bits below the partition
/// bits. They are sorted in parallel and later written in parallel.
pub const BUCKET_BITS: u32 = 8;
pub const BUCKETS: usize = 1 << BUCKET_BITS;

/// Records staged per bucket by each thread before they are appended to the
/// shared bucket.
const STAGE: usize = 1024;

/// The hash prefix that is the key of a group in the cold_positions column
/// family. Memory partitions and buckets are determined by this prefix
/// only, so that all keys of a group are written by the same thread, in one
/// SST file.
fn hash_prefix(key: &[u8; IntermediateEntry::KEY_SIZE]) -> u32 {
    u32::from_be_bytes(key[..ColdKey::PREFIX_SIZE].try_into().expect("cold prefix"))
}

/// The memory partition of a key, and the position of the key within the
/// partition (multiply-shift: the integer and the fractional part of the
/// scaled hash prefix). Together they are in key order.
#[inline]
fn split(key: &[u8; IntermediateEntry::KEY_SIZE], memory_partitions: usize) -> (usize, u32) {
    let scaled = u64::from(hash_prefix(key)) * memory_partitions as u64;
    ((scaled >> 32) as usize, scaled as u32)
}

/// The memory partition of a key.
#[inline]
pub fn memory_partition(
    key: &[u8; IntermediateEntry::KEY_SIZE],
    memory_partitions: usize,
) -> usize {
    split(key, memory_partitions).0
}

/// The bucket of a key within its memory partition.
#[inline]
pub fn bucket(key: &[u8; IntermediateEntry::KEY_SIZE], memory_partitions: usize) -> usize {
    (split(key, memory_partitions).1 >> (32 - BUCKET_BITS)) as usize
}

/// Records of a memory partition, distributed to buckets in key order.
pub struct Partition {
    pub buckets: Vec<Vec<IntermediateEntry>>,
}

impl Partition {
    pub fn records(&self) -> usize {
        self.buckets.iter().map(Vec::len).sum()
    }

    /// Sorts the buckets in parallel. Afterwards all records of the
    /// partition are in order.
    pub fn sort(&mut self, threads: usize) {
        let buckets = Mutex::new(self.buckets.iter_mut());
        thread::scope(|s| {
            for _ in 0..threads.clamp(1, BUCKETS) {
                s.spawn(|| {
                    loop {
                        let Some(bucket) = buckets.lock().expect("lock buckets").next() else {
                            break;
                        };
                        bucket.sort_unstable_by_key(IntermediateEntry::sort_key);
                    }
                });
            }
        });
    }
}

/// Distributes records to shared buckets, staging them per thread.
struct Bucketer<'a> {
    memory_partitions: usize,
    buckets: &'a [Mutex<Vec<IntermediateEntry>>],
    staged: Vec<Vec<IntermediateEntry>>,
}

impl<'a> Bucketer<'a> {
    fn new(memory_partitions: usize, buckets: &'a [Mutex<Vec<IntermediateEntry>>]) -> Bucketer<'a> {
        Bucketer {
            memory_partitions,
            buckets,
            staged: (0..BUCKETS).map(|_| Vec::with_capacity(STAGE)).collect(),
        }
    }

    #[inline]
    fn push(&mut self, entry: IntermediateEntry) {
        let b = bucket(entry.key(), self.memory_partitions);
        let staged = &mut self.staged[b];
        staged.push(entry);
        if staged.len() >= STAGE {
            self.buckets[b].lock().expect("lock bucket").append(staged);
        }
    }

    fn flush(&mut self) {
        for (b, staged) in self.staged.iter_mut().enumerate() {
            if !staged.is_empty() {
                self.buckets[b].lock().expect("lock bucket").append(staged);
            }
        }
    }
}

/// Shared buckets with capacity for about `records` uniformly distributed
/// records.
fn shared_buckets(records: usize) -> Vec<Mutex<Vec<IntermediateEntry>>> {
    let capacity = records / BUCKETS + records / BUCKETS / 16 + 2 * STAGE;
    (0..BUCKETS)
        .map(|_| Mutex::new(Vec::with_capacity(capacity)))
        .collect()
}

fn into_partition(buckets: Vec<Mutex<Vec<IntermediateEntry>>>) -> Partition {
    Partition {
        buckets: buckets
            .into_iter()
            .map(|bucket| bucket.into_inner().expect("bucket"))
            .collect(),
    }
}

/// Loads a scratch file in parallel, distributing its records to buckets.
pub fn load(
    scratch: &ScratchFile,
    records: usize,
    codec: ScratchCodec,
    memory_partitions: usize,
    threads: usize,
) -> Partition {
    let buckets = shared_buckets(records);
    let bucketers = scratch.load(
        threads,
        || Bucketer::new(memory_partitions, &buckets),
        |bucketer, _, raw| codec.decode(raw, |entry| bucketer.push(entry)),
    );
    for mut bucketer in bucketers {
        bucketer.flush();
    }
    into_partition(buckets)
}

/// Distributes the records collected by multiple threads to buckets, in
/// parallel. Each part is freed as soon as it is distributed.
pub fn from_parts(
    parts: Vec<Vec<IntermediateEntry>>,
    memory_partitions: usize,
    threads: usize,
) -> Partition {
    let records = parts.iter().map(Vec::len).sum();
    let buckets = shared_buckets(records);
    let parts = Mutex::new(parts);
    thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                let mut bucketer = Bucketer::new(memory_partitions, &buckets);
                loop {
                    let Some(part) = parts.lock().expect("lock parts").pop() else {
                        break;
                    };
                    for entry in part {
                        bucketer.push(entry);
                    }
                }
                bucketer.flush();
            });
        }
    });
    into_partition(buckets)
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_buckets_in_key_order() {
        arbtest(|u| {
            let memory_partitions: usize = u.int_in_range(1..=5000)?;
            let p = u.int_in_range(0..=memory_partitions - 1)?;
            // Prefixes mapped to memory partition p.
            let lo = ((p as u64) << 32).div_ceil(memory_partitions as u64);
            let hi = (((p + 1) as u64) << 32).div_ceil(memory_partitions as u64);
            let mut records: Vec<IntermediateEntry> = Vec::new();
            for _ in 0..u.int_in_range(0..=2000)? {
                let mut bytes: [u8; IntermediateEntry::SIZE] = u.arbitrary()?;
                let prefix = (lo + u.int_in_range(0..=hi - lo - 1)?) as u32;
                bytes[..ColdKey::PREFIX_SIZE].copy_from_slice(&prefix.to_be_bytes());
                assert_eq!(
                    memory_partition(IntermediateEntry(bytes).key(), memory_partitions),
                    p
                );
                records.push(IntermediateEntry(bytes));
            }
            let parts = records.chunks(7).map(<[_]>::to_vec).collect();
            let mut partition = from_parts(parts, memory_partitions, 3);
            partition.sort(3);
            let mut actual: Vec<IntermediateEntry> = partition.buckets.concat();
            assert!(actual.is_sorted_by_key(IntermediateEntry::sort_key));

            actual.sort_unstable_by_key(|entry| entry.0);
            records.sort_unstable_by_key(|entry| entry.0);
            assert_eq!(actual, records);
            Ok(())
        });
    }
}
