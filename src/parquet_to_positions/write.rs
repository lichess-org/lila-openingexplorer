use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use rocksdb::Options;

use crate::{
    intermediate::{EntryPacker, IntermediateEntry},
    packed::{ColdGroup, ColdKey, PositionKey},
    parquet_to_positions::{
        hot::HotEntry,
        partition::{BUCKETS, Partition, bucket},
    },
    sst::{RollingSstWriter, SstError, SstStats},
};

/// Subdirectory of the output for the cold positions column family.
pub const COLD_DIR: &str = "cold_positions";

/// Subdirectory of the output for the positions column family.
pub const POSITIONS_DIR: &str = "positions";

/// Ranges per SST thread and column family, to balance the load.
const RANGES_PER_THREAD: usize = 4;

/// Hard limit for SST files, as a multiple of the target size, in case the
/// size estimate of a range is far off.
const MAX_FILE_SIZE_FACTOR: u64 = 2;

/// Initial guesses for SST bytes per record (of both column families),
/// used until measured. From the full run at 50 plies.
const INITIAL_COLD_BYTES_PER_RECORD: f64 = 4.3;
const INITIAL_POSITIONS_BYTES_PER_RECORD: f64 = 3.8;

pub struct SstContext<'a> {
    pub cold_opts: &'a Options,
    pub positions_opts: &'a Options,
    pub out: &'a Path,
    /// Approximate size of the SST files.
    pub target_size: u64,
    /// Write the cold positions column family.
    pub cold: bool,
    pub stats: Mutex<OutputStats>,
}

#[derive(Debug, Default, Clone)]
pub struct OutputStats {
    /// Cold groups are the keys.
    pub cold: SstStats,
    pub positions: SstStats,
    /// Records written as cold entries.
    pub cold_records: u64,
    /// Records written as part of a positions entry, excluding hot records.
    pub position_records: u64,
    /// Records of positions reached by a single game that were not written,
    /// because cold positions are disabled.
    pub skipped_cold_records: u64,
}

impl OutputStats {
    /// SST bytes of each column family per record written so far
    /// (excluding hot records).
    fn bytes_per_record(&self) -> (f64, f64) {
        let records = self.cold_records + self.skipped_cold_records + self.position_records;
        if records == 0 {
            (
                INITIAL_COLD_BYTES_PER_RECORD,
                INITIAL_POSITIONS_BYTES_PER_RECORD,
            )
        } else {
            (
                self.cold.bytes as f64 / records as f64,
                self.positions.bytes as f64 / records as f64,
            )
        }
    }

    fn merge(&mut self, other: &OutputStats) {
        self.cold.merge(&other.cold);
        self.positions.merge(&other.positions);
        self.cold_records += other.cold_records;
        self.position_records += other.position_records;
        self.skipped_cold_records += other.skipped_cold_records;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Family {
    Cold,
    Positions,
}

/// Writes a sorted partition and the packed hot entries of its key range
/// to SST files for both column families (or only positions, unless
/// `sst.cold`).
///
/// For each column family, the partition is split into contiguous ranges of
/// buckets, at most one target file size each, or more if that would exceed
/// the number of parallel tasks. Each range is split by record count into
/// equal files of at most the target size, so that there are no small
/// leftover files. All ranges are written in parallel. File names sort in key order.
pub fn write_partition(
    partition: &Partition,
    hot: &[HotEntry],
    memory_partition: usize,
    memory_partitions: usize,
    sst: &SstContext<'_>,
    threads: usize,
) {
    let (cold_per_record, positions_per_record) =
        sst.stats.lock().expect("lock stats").bytes_per_record();
    let records = partition.records() as f64;
    let hot_bytes: usize = hot.iter().map(|(key, value)| key.len() + value.len()).sum();
    let target_size = sst.target_size as f64;
    let ranges = |estimated_bytes: f64| {
        let ranges = ((estimated_bytes / target_size).ceil() as usize)
            .clamp(1, (threads * RANGES_PER_THREAD).min(BUCKETS));
        let files = ((estimated_bytes / ranges as f64 / target_size).ceil() as usize).max(1);
        (ranges, files)
    };
    let (positions_ranges, positions_files) =
        ranges(records * positions_per_record + hot_bytes as f64);
    let (cold_ranges, cold_files) = if sst.cold {
        ranges(records * cold_per_record)
    } else {
        (0, 0)
    };
    // The larger positions tasks first.
    let tasks: Vec<(Family, usize, usize, usize)> = (0..positions_ranges)
        .map(|range| (Family::Positions, range, positions_ranges, positions_files))
        .chain((0..cold_ranges).map(|range| (Family::Cold, range, cold_ranges, cold_files)))
        .collect();
    let next_task = AtomicUsize::new(0);

    thread::scope(|s| {
        for _ in 0..threads.clamp(1, tasks.len()) {
            s.spawn(|| {
                let mut packer = EntryPacker::default();
                let mut buf = Vec::new();
                while let Some(&(family, range, ranges, files)) =
                    tasks.get(next_task.fetch_add(1, Ordering::Relaxed))
                {
                    let buckets = range * BUCKETS / ranges..(range + 1) * BUCKETS / ranges;
                    let hot_start = hot
                        .partition_point(|(key, _)| bucket(key, memory_partitions) < buckets.start);
                    let hot_end = hot
                        .partition_point(|(key, _)| bucket(key, memory_partitions) < buckets.end);
                    let stats = write_range(
                        family,
                        &partition.buckets[buckets],
                        &hot[hot_start..hot_end],
                        files,
                        sst,
                        &format!("{memory_partition:05}_{range:05}"),
                        &mut packer,
                        &mut buf,
                    )
                    .expect("write sst files");
                    sst.stats.lock().expect("lock stats").merge(&stats);
                }
            });
        }
    });
}

fn hash_bytes(entry: &IntermediateEntry) -> &[u8] {
    &entry.0[..PositionKey::SIZE]
}

/// Writes a cold group, which is empty afterwards.
fn put_cold_group(
    writer: &mut RollingSstWriter<'_>,
    prefix: &[u8; ColdKey::PREFIX_SIZE],
    group: &mut ColdGroup,
    buf: &mut Vec<u8>,
) -> Result<(), SstError> {
    buf.clear();
    group.pack_entire(buf);
    group.clear();
    writer.put(prefix, buf)
}

/// Writes the keys of one column family for sorted buckets and sorted hot
/// entries of one key range, split into the given number of files.
#[allow(clippy::too_many_arguments)]
fn write_range(
    family: Family,
    buckets: &[Vec<IntermediateEntry>],
    hot: &[HotEntry],
    files: usize,
    sst: &SstContext<'_>,
    name: &str,
    packer: &mut EntryPacker,
    buf: &mut Vec<u8>,
) -> Result<OutputStats, SstError> {
    let (dir, opts) = match family {
        Family::Cold => (COLD_DIR, sst.cold_opts),
        Family::Positions => (POSITIONS_DIR, sst.positions_opts),
    };
    let mut writer = RollingSstWriter::new(
        opts,
        sst.out.join(dir).join(name),
        MAX_FILE_SIZE_FACTOR * sst.target_size,
    );
    // Start the next file after this many records.
    let total_records: usize = buckets.iter().map(Vec::len).sum();
    let mut file = 1;
    let mut next_file_at = total_records / files;
    let mut records = 0;
    let mut cold_records = 0;
    // The cold group being collected, and the prefix of its key.
    let mut cold_group = ColdGroup::new();
    let mut cold_prefix = [0; ColdKey::PREFIX_SIZE];
    let mut cold_value = Vec::new();
    let mut position_records = 0;
    let mut skipped_cold_records = 0;
    let mut hot = hot.iter().peekable();

    for run in buckets
        .iter()
        .flat_map(|bucket| bucket.chunk_by(|a, b| hash_bytes(a) == hash_bytes(b)))
    {
        if records >= next_file_at && file < files {
            // Before the next put, so that cold groups stay whole.
            writer.finish_file()?;
            file += 1;
            next_file_at = total_records * file / files;
        }
        records += run.len();
        let hash = hash_bytes(&run[0]);
        match (family, run) {
            (Family::Cold, [single]) => {
                let key = ColdKey::from_hash_bytes(hash);
                if *key.prefix() != cold_prefix && !cold_group.is_empty() {
                    put_cold_group(&mut writer, &cold_prefix, &mut cold_group, &mut cold_value)?;
                }
                cold_prefix = *key.prefix();
                cold_group.add(key.suffix(), single.game_index().get());
                cold_records += 1;
            }
            (Family::Cold, _) => (),
            (Family::Positions, [_]) => {
                if !sst.cold {
                    skipped_cold_records += 1;
                }
            }
            (Family::Positions, _) => {
                while let Some((key, value)) =
                    hot.next_if(|(key, _)| key[..PositionKey::SIZE] < *hash)
                {
                    writer.put(key, value)?;
                }
                debug_assert!(
                    hot.peek()
                        .is_none_or(|(key, _)| key[..PositionKey::SIZE] != *hash),
                    "hot hash in partition records"
                );
                packer.pack_sorted(run, buf);
                writer.put(run[0].key(), buf)?;
                position_records += run.len() as u64;
            }
        }
    }
    if !cold_group.is_empty() {
        put_cold_group(&mut writer, &cold_prefix, &mut cold_group, &mut cold_value)?;
    }
    if family == Family::Positions {
        for (key, value) in hot {
            writer.put(key, value)?;
        }
    }

    let written = writer.finish()?;
    Ok(match family {
        Family::Cold => OutputStats {
            cold: written,
            cold_records,
            ..OutputStats::default()
        },
        Family::Positions => OutputStats {
            positions: written,
            position_records,
            skipped_cold_records,
            ..OutputStats::default()
        },
    })
}
