use std::{
    cmp::Reverse,
    collections::HashSet,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use nohash_hasher::IntMap;
use rustc_hash::FxHashMap;

use crate::{
    intermediate::IntermediateEntry,
    packed::{GameIndex, LichessEntry, Month, PositionKey},
    parquet::Input,
    parquet_to_positions::replay::Replayer,
};

/// Only positions within the first plies of the sampled games are
/// considered as candidates for hot positions. This limits the cost of
/// sampling, not correctness: hot positions are routed by hash at any ply.
const SAMPLE_PLIES: usize = 16;

/// Row groups to replay for the estimate.
const SAMPLE_ROW_GROUPS: usize = 32;

/// A position is never considered hot with fewer sampled records.
const MIN_SAMPLE_COUNT: u64 = 64;

const MAX_HOT: usize = 256;

/// Estimated memory per month of a hot position.
pub const HOT_KEY_MEMORY: u64 = 64 << 10;

/// Statistics from replaying a sample of the input.
pub struct Sample {
    pub sampled_rows: u64,
    /// Records of the valid sampled games.
    pub records: u64,
    /// Distinct months of the valid sampled games.
    pub months: u64,
    /// Records per hash, within the first `SAMPLE_PLIES` of each game.
    counts: Vec<(PositionKey, u64)>,
}

impl Sample {
    pub fn replay(input: &Input, threads: usize) -> Sample {
        let num_row_groups = input.num_row_groups();
        let samples = SAMPLE_ROW_GROUPS.min(num_row_groups);
        let next = AtomicUsize::new(0);
        let results = Mutex::new(Vec::new());

        thread::scope(|s| {
            for _ in 0..threads.clamp(1, samples.max(1)) {
                s.spawn(|| {
                    let mut replayer = Replayer::new();
                    let mut counts: IntMap<PositionKey, u64> = IntMap::default();
                    let mut months: HashSet<Month> = HashSet::new();
                    let (mut rows, mut records) = (0, 0);
                    // Game indexes do not matter for the sample.
                    let index = GameIndex::new(0).expect("game index");
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        if k >= samples {
                            break;
                        }
                        let i = k * num_row_groups / samples;
                        let batch = input.read_games(i).expect("read row group");
                        for (_, game) in batch.rows() {
                            rows += 1;
                            let info = replayer.replay_game(&game, index);
                            months.insert(info.month);
                            records += replayer.len() as u64;
                            for hash in replayer.hashes_before(SAMPLE_PLIES) {
                                *counts
                                    .entry(PositionKey::from_zobrist(hash, info.variant))
                                    .or_default() += 1;
                            }
                        }
                    }
                    results
                        .lock()
                        .expect("lock results")
                        .push((rows, records, months, counts));
                });
            }
        });

        let (mut sampled_rows, mut records) = (0, 0);
        let mut months = HashSet::new();
        let mut counts: IntMap<PositionKey, u64> = IntMap::default();
        for (thread_rows, thread_records, thread_months, thread_counts) in
            results.into_inner().expect("results")
        {
            sampled_rows += thread_rows;
            records += thread_records;
            months.extend(thread_months);
            for (hash, count) in thread_counts {
                *counts.entry(hash).or_default() += count;
            }
        }
        let mut counts: Vec<(PositionKey, u64)> = counts
            .into_iter()
            .filter(|&(_, count)| count >= MIN_SAMPLE_COUNT)
            .collect();
        counts.sort_unstable_by_key(|&(hash, count)| (Reverse(count), hash));
        Sample {
            sampled_rows,
            records,
            months: months.len() as u64,
            counts,
        }
    }

    /// Hashes with at least `threshold` estimated records in an input of
    /// `rows` rows, most frequent first, and their estimated records.
    pub fn hot(&self, rows: u64, threshold: u64) -> Vec<(PositionKey, u64)> {
        let scale = rows as f64 / self.sampled_rows as f64;
        self.counts
            .iter()
            .map(|&(hash, count)| (hash, (count as f64 * scale) as u64))
            .take_while(|&(_, estimated)| estimated >= threshold)
            .take(MAX_HOT)
            .collect()
    }
}

/// Shards of the shared hot map, to reduce lock contention.
const SHARDS: usize = 64;

/// Hot records buffered by each thread before they are added to the shared
/// map.
pub const HOT_BATCH: usize = 1 << 16;

/// A packed entry of a hot position, with its `IntermediateEntry::key()`.
pub type HotEntry = ([u8; IntermediateEntry::KEY_SIZE], Vec<u8>);

/// Packed entries of hot positions, sorted by key.
pub type HotEntries = Vec<HotEntry>;

/// Aggregated entries of hot positions, keyed by `IntermediateEntry::key()`.
pub struct HotMap {
    shards: Vec<Mutex<FxHashMap<[u8; IntermediateEntry::KEY_SIZE], LichessEntry>>>,
}

impl Default for HotMap {
    fn default() -> HotMap {
        HotMap {
            shards: (0..SHARDS).map(|_| Mutex::default()).collect(),
        }
    }
}

impl HotMap {
    fn shard(entry: &IntermediateEntry) -> usize {
        // The prefix also selects the memory partition, so take low bits.
        (entry.hash_prefix() as usize) % SHARDS
    }

    /// Adds a batch of records. Leaves the batch empty.
    fn add(&self, batch: &mut Vec<IntermediateEntry>) {
        batch.sort_unstable_by_key(|entry| (Self::shard(entry), *entry.key()));
        for shard_run in batch.chunk_by(|a, b| Self::shard(a) == Self::shard(b)) {
            let mut shard = self.shards[Self::shard(&shard_run[0])]
                .lock()
                .expect("lock hot shard");
            for key_run in shard_run.chunk_by(|a, b| a.key() == b.key()) {
                let lichess_entry = shard.entry(*key_run[0].key()).or_default();
                for entry in key_run {
                    lichess_entry.add_record(entry);
                }
            }
        }
        batch.clear();
    }

    pub fn pack(self) -> HotEntries {
        let mut packed: HotEntries = self
            .shards
            .into_iter()
            .flat_map(|shard| shard.into_inner().expect("hot shard"))
            .map(|(key, mut lichess_entry)| {
                let mut buf = Vec::new();
                lichess_entry.pack_entire(&mut buf);
                (key, buf)
            })
            .collect();
        packed.sort_unstable_by_key(|(key, _)| *key);
        packed
    }
}

/// Buffers hot records of one thread for a `HotMap`.
pub struct HotBuffer<'a> {
    map: &'a HotMap,
    batch: Vec<IntermediateEntry>,
}

impl<'a> HotBuffer<'a> {
    pub fn new(map: &'a HotMap) -> HotBuffer<'a> {
        HotBuffer {
            map,
            batch: Vec::with_capacity(HOT_BATCH),
        }
    }

    #[inline]
    pub fn push(&mut self, entry: IntermediateEntry) {
        self.batch.push(entry);
        if self.batch.len() >= HOT_BATCH {
            self.map.add(&mut self.batch);
        }
    }
}

impl Drop for HotBuffer<'_> {
    fn drop(&mut self) {
        self.map.add(&mut self.batch);
    }
}
