mod hot;
mod partition;
mod replay;
mod scratch_codec;
mod write;

use std::{
    fmt, io, mem,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam::channel;
use indicatif::{MultiProgress, ProgressBar};
use nohash_hasher::IntSet;
pub use write::{COLD_DIR, OutputStats, POSITIONS_DIR};

use crate::{
    intermediate::IntermediateEntry,
    packed::{ColdKey, GameIndex, PositionKey},
    parquet::{Input, PrefetchTimes, Prefetcher},
    parquet_to_positions::{
        hot::{HOT_BATCH, HOT_KEY_MEMORY, HotBuffer, HotEntries, HotMap, Sample},
        partition::{Partition, memory_partition},
        replay::Replayer,
        scratch_codec::ScratchCodec,
        write::{SstContext, write_partition},
    },
    scatter::{
        ChunkWriterConfig, ChunkWriterTimes, Layout, RESIDENT_PARTITIONS_PER_PASS, ScratchFile,
        spawn_chunk_writers,
    },
    sst::SstConfig,
    util::{Gauge, create_dir, create_empty_dir, log_above, progress_bar, timed, timed_seconds},
};

/// Partitions in memory at the same time: one being written to SST files,
/// one being loaded and sorted, and the resident partition of the next pass,
/// which is temporarily held twice while it is distributed to buckets.
const PARTITIONS_IN_MEMORY: u64 = 4;

/// Zstd level of cold positions SST files, given the estimated records.
/// Cold groups are practically incompressible, unless there are fewer
/// records than cold key prefixes: then most groups have a single item, so
/// the keys and the headers of the values dominate (masters cold positions
/// compress to 54%).
fn cold_zstd_level(records: u64) -> i32 {
    if records < 1 << (8 * ColdKey::PREFIX_SIZE) {
        3
    } else {
        0
    }
}

/// Assumed ratio of zstd compressed scratch chunks to their raw bytes
/// (encoded with `ScratchCodec`). Measured with zstd level 1 about 0.88 at
/// 122 partitions per pass; zstd gains little after the codec removed most
/// redundancy. The estimate has to stay safe for the first pass, which is
/// not throttled by the scratch budget.
const SCRATCH_RATIO: f64 = 0.95;

/// Ratio of scratch chunks to their raw bytes for the given zstd level.
fn scratch_ratio(zstd_level: i32) -> f64 {
    if zstd_level == 0 { 1.0 } else { SCRATCH_RATIO }
}

/// Upper bound for the SST bytes per record, without the fixed costs of
/// cold groups. Measured on lichess 5.4 (positions) and 5.7 (cold) at full
/// scale, less on slices, and 6.2 for masters positions.
const OUTPUT_RECORD_BYTES: f64 = 6.5;

/// Upper bound for the SST bytes of a cold group in addition to its records:
/// its key and value header, uncompressed.
const COLD_GROUP_BYTES: u64 = 15;

/// Upper bound for the bytes of all SST files. Cold groups have a fixed
/// cost, so inputs with few records per group need more bytes per record
/// (measured up to 11.7).
fn output_bound(records: u64) -> u64 {
    let cold_groups = records.min(1 << (8 * ColdKey::PREFIX_SIZE));
    (records as f64 * OUTPUT_RECORD_BYTES) as u64 + cold_groups * COLD_GROUP_BYTES
}

/// Fixed memory overhead for the parquet reader, rocksdb and allocator slack.
const BASE_MEMORY: u64 = 4 << 30;

/// A position is hot if it is estimated to have at least this fraction of
/// the records of a memory partition.
const HOT_FRACTION: u64 = 32;

const CHUNK_RECORDS: usize = 32 * 1024;

const CHUNK_BYTES: usize = CHUNK_RECORDS * IntermediateEntry::SIZE;

/// Records of one memory partition, buffered before they are compressed.
type Chunk = Vec<[u8; IntermediateEntry::SIZE]>;

fn new_chunk() -> Chunk {
    Vec::with_capacity(CHUNK_RECORDS)
}

/// Estimated data sizes, from replaying a sample of row groups.
struct Estimate {
    rows: u64,
    /// Records of all games, excluding records of hot positions.
    records: u64,
    /// Hot positions and their estimated records.
    hot: Vec<(PositionKey, u64)>,
    months: u64,
    /// Ratio of scratch chunks to their raw bytes.
    scratch_ratio: f64,
    /// Scratch space to use, leaving room for the output where needed.
    scratch_budget: u64,
}

impl Estimate {
    fn new(input: &Input, config: &Config, read_threads: u64) -> io::Result<Estimate> {
        let rows = input.total_rows();
        let sample = Sample::replay(input, config.threads);
        let per_row = if sample.sampled_rows == 0 {
            0.0
        } else {
            sample.records as f64 / sample.sampled_rows as f64
        };
        let mut estimate = Estimate {
            rows,
            // With a safety margin for sampling error.
            records: (per_row * rows as f64 * 1.05) as u64,
            hot: Vec::new(),
            months: sample.months,
            scratch_ratio: scratch_ratio(config.scratch_zstd_level),
            scratch_budget: 0,
        };
        estimate.scratch_budget = config.scratch_budget.resolve(estimate.output())?;
        // Choose hot positions for the layout without them. Then the
        // remaining records may fit into fewer partitions.
        let layout = estimate.layout(config, read_threads);
        let threshold = estimate.records / layout.memory_partitions() as u64 / HOT_FRACTION;
        estimate.hot = sample.hot(rows, threshold.max(1));
        estimate.records = estimate.records.saturating_sub(estimate.hot_records());
        Ok(estimate)
    }

    /// Upper bound for the bytes of all SST files.
    fn output(&self) -> u64 {
        output_bound(self.records + self.hot_records())
    }

    fn record_bytes(&self) -> u64 {
        self.records * IntermediateEntry::SIZE as u64
    }

    fn hot_records(&self) -> u64 {
        self.hot.iter().map(|(_, records)| records).sum()
    }

    /// Bytes of one memory partition in memory, including bucket slack.
    fn partition_memory(&self, layout: &Layout) -> u64 {
        self.record_bytes() * 17 / 16 / layout.memory_partitions() as u64
    }

    fn hot_memory(&self, read_threads: u64) -> u64 {
        self.hot.len() as u64 * self.months.max(1) * HOT_KEY_MEMORY
            + read_threads * (HOT_BATCH * IntermediateEntry::SIZE) as u64
    }

    /// Peak memory for the given layout and read threads.
    fn memory(&self, layout: &Layout, read_threads: u64) -> u64 {
        let buffers =
            read_threads * layout.scratch_partitions_per_pass() as u64 * CHUNK_BYTES as u64;
        BASE_MEMORY
            + PARTITIONS_IN_MEMORY * self.partition_memory(layout)
            + buffers
            + self.hot_memory(read_threads)
    }

    /// Bytes of a record in a scratch chunk, before compression.
    fn scratch_record_bytes(&self, layout: &Layout) -> f64 {
        let rate =
            self.records as f64 / self.rows.max(1) as f64 / layout.memory_partitions() as f64;
        ScratchCodec::new(0, layout.memory_partitions()).record_bytes(rate)
    }

    /// Scratch space needed by one pass.
    fn scratch(&self, layout: &Layout) -> u64 {
        let per_partition =
            self.records as f64 * self.scratch_record_bytes(layout) * self.scratch_ratio
                / layout.memory_partitions() as f64;
        (per_partition * layout.scratch_partitions_per_pass() as f64) as u64
    }

    fn layout(&self, config: &Config, read_threads: u64) -> Layout {
        if let Some((passes, per_pass)) = config.layout {
            return Layout::new(passes, per_pass);
        }
        Layout::choose(|layout| {
            (
                self.memory(layout, read_threads) <= config.memory_budget,
                self.scratch(layout) <= self.scratch_budget,
            )
        })
    }
}

/// Counters shared by the threads of one run.
#[derive(Default)]
struct Counters {
    /// Valid games (counted in the first pass).
    games: AtomicU64,
    /// Records routed to memory partitions.
    records: AtomicU64,
    hot_records: AtomicU64,
    /// Bytes currently occupied by scratch files.
    scratch_usage: Gauge,
    /// Bytes of scratch files of previous passes that are still to be
    /// loaded and removed.
    reclaimable_scratch: AtomicU64,
    /// Partitions written to SST files so far.
    written_partitions: Gauge,
}

impl Counters {
    /// Removes a scratch file of a previous pass.
    fn remove_scratch(&self, file: ScratchFile) -> io::Result<()> {
        // Before the usage changes, so that throttled chunk writers see both.
        self.reclaimable_scratch
            .fetch_sub(file.len(), Ordering::Release);
        file.remove(&self.scratch_usage)
    }
}

/// Scratch space to use. Determines the number of passes over the input.
pub enum ScratchBudget {
    Bytes(u64),
    /// 90% of the free bytes of the file system of the scratch directory.
    /// If the output is written to the same file system, after reserving
    /// its estimated size.
    Free {
        bytes: u64,
        shared_with_output: bool,
    },
}

impl ScratchBudget {
    fn resolve(&self, output: u64) -> io::Result<u64> {
        let free = match *self {
            ScratchBudget::Bytes(bytes) => return Ok(bytes),
            ScratchBudget::Free {
                bytes,
                shared_with_output: false,
            } => bytes,
            ScratchBudget::Free {
                bytes,
                shared_with_output: true,
            } => bytes.checked_sub(output).ok_or_else(|| {
                io::Error::other(format!(
                    "up to {} GiB output may not fit into the {} GiB free on the file system shared with the scratch directory (override with --scratch-budget)",
                    output.div_ceil(1 << 30),
                    bytes >> 30,
                ))
            })?,
        };
        Ok(free / 10 * 9)
    }
}

pub struct Config {
    /// Threads to use in total, unless overridden for a stage.
    pub threads: usize,
    /// Maximum bytes of memory to use. Determines the number of memory
    /// partitions.
    pub memory_budget: u64,
    pub scratch_budget: ScratchBudget,
    /// Threads for replaying the input.
    pub read_threads: Option<usize>,
    /// Threads for compressing and writing scratch chunks.
    pub write_threads: Option<usize>,
    /// Threads for loading and sorting partitions.
    pub load_threads: Option<usize>,
    /// Threads for writing SST files.
    pub sst_threads: Option<usize>,
    /// Concurrent reads of the input, ahead of the decoding threads.
    pub io_depth: usize,
    /// Use O_DIRECT to read the input and the scratch files.
    pub direct_reads: bool,
    /// Zstd level for scratch chunks, or 0 for no compression.
    pub scratch_zstd_level: i32,
    /// Skip the first row groups of the inputs.
    pub skip_row_groups: usize,
    /// Only process the first row groups of the inputs.
    pub row_groups: Option<usize>,
    /// Stop after the scatter of the first pass.
    pub scatter_only: bool,
    /// Scan the input for the next pass while partitions of the previous
    /// pass are still being written.
    pub overlap: bool,
    /// Start a new SST file once the current one reaches this size.
    pub sst_target_size: u64,
    /// Write the cold positions (reached by a single game).
    pub cold: bool,
    /// Passes and memory partitions per pass, instead of choosing them by
    /// the budgets (for experiments).
    pub layout: Option<(usize, usize)>,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub games: u64,
    /// Records of all games, including hot records.
    pub records: u64,
    pub hot_positions: usize,
    pub hot_records: u64,
    pub output: OutputStats,
}

/// The input, its estimated size, and the layout and threads chosen to
/// convert it.
pub struct Plan {
    input: Input,
    estimate: Estimate,
    layout: Layout,
    read_threads: usize,
    write_threads: usize,
    load_threads: usize,
    sst_threads: usize,
    /// Threads for loading and writing SST files once the input has been
    /// scanned for the last time.
    final_load_threads: usize,
    final_sst_threads: usize,
}

impl Plan {
    pub fn new(parquet: &[PathBuf], config: &Config) -> io::Result<Plan> {
        let threads = config.threads;
        let input = Input::open(
            parquet,
            config.skip_row_groups,
            config.row_groups,
            config.direct_reads,
        )?;

        // The layout depends on the read threads (buffers), which depend on
        // the layout (overlap). Resolve with the larger read thread count.
        let max_read_threads = config.read_threads.unwrap_or(threads * 2 / 3).max(1);
        let estimate = Estimate::new(&input, config, max_read_threads as u64)?;
        let layout = estimate.layout(config, max_read_threads as u64);
        // With a single pass the scan and the partition writes never overlap,
        // so each phase can use all threads. With several passes the scan of
        // the next pass runs while partitions are written.
        // Replaying is about 2.5 times the work of compressing chunks, and
        // loading and sorting about 2/3 of the work of writing SST files.
        let single_pass = layout.passes() == 1 || !config.overlap;
        let (final_load_default, final_sst_default) = (threads * 2 / 5, threads * 3 / 5);
        let (read_default, write_default, load_default, sst_default) = if single_pass {
            (
                threads * 2 / 3,
                threads / 3,
                final_load_default,
                final_sst_default,
            )
        } else {
            (threads / 2, threads / 5, threads / 5, threads / 4)
        };

        Ok(Plan {
            input,
            estimate,
            layout,
            read_threads: config.read_threads.unwrap_or(read_default).max(1),
            write_threads: config.write_threads.unwrap_or(write_default).max(1),
            load_threads: config.load_threads.unwrap_or(load_default).max(1),
            sst_threads: config.sst_threads.unwrap_or(sst_default).max(1),
            final_load_threads: config.load_threads.unwrap_or(final_load_default).max(1),
            final_sst_threads: config.sst_threads.unwrap_or(final_sst_default).max(1),
        })
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} games, about {} records ({} GiB) and {} records of {} hot positions; {} passes with {} memory partitions each, about {} GiB scratch per pass (budget {} GiB), {} GiB memory and up to {} GiB output; threads: {} read, {} write, {} load ({} after the last scan), {} sst ({} after the last scan)",
            self.estimate.rows,
            self.estimate.records,
            self.estimate.record_bytes() >> 30,
            self.estimate.hot_records(),
            self.estimate.hot.len(),
            self.layout.passes(),
            self.layout.memory_partitions_per_pass(),
            self.estimate.scratch(&self.layout) >> 30,
            self.estimate.scratch_budget >> 30,
            self.estimate.memory(&self.layout, self.read_threads as u64) >> 30,
            self.estimate.output().div_ceil(1 << 30),
            self.read_threads,
            self.write_threads,
            self.load_threads,
            self.final_load_threads,
            self.sst_threads,
            self.final_sst_threads,
        )
    }
}

/// Source of the records of a memory partition.
enum PartitionSource {
    /// Collected in memory by the scan threads.
    Resident(Vec<Vec<IntermediateEntry>>),
    /// A scratch file and its number of records.
    Scratch(ScratchFile, u64),
}

struct PartitionJob {
    memory_partition: usize,
    source: PartitionSource,
    hot: HotEntries,
}

/// Indexes the positions of the planned input into SST files in
/// `out/cold_positions` and `out/positions`, using `scratch_dir` for
/// temporary partition files.
pub fn convert(
    plan: &Plan,
    scratch_dir: &Path,
    out: &Path,
    config: &Config,
    progress: &MultiProgress,
) -> io::Result<Stats> {
    let Plan {
        ref input,
        ref estimate,
        ref layout,
        read_threads,
        write_threads,
        load_threads,
        sst_threads,
        final_load_threads,
        final_sst_threads,
    } = *plan;

    // Fail early rather than after the first scatter pass.
    create_dir(scratch_dir)?;
    create_dir(out)?;
    if config.cold {
        create_empty_dir(&out.join(COLD_DIR))?;
    }
    create_empty_dir(&out.join(POSITIONS_DIR))?;

    let hot_set: IntSet<PositionKey> = estimate.hot.iter().map(|(hash, _)| *hash).collect();
    let sst = SstContext {
        cold_opts: &SstConfig {
            zstd_level: cold_zstd_level(estimate.records),
            ..SstConfig::COLD_POSITIONS
        }
        .options(),
        positions_opts: &SstConfig::POSITIONS.options(),
        out,
        target_size: config.sst_target_size,
        cold: config.cold,
        stats: Mutex::new(OutputStats::default()),
    };
    let counters = Counters::default();
    // Set once the input has been scanned for the last time, so that loading
    // and writing can use all threads.
    let scanned = AtomicBool::new(false);

    let progress = progress.add(progress_bar(estimate.rows * layout.passes() as u64));

    thread::scope(|s| {
        let progress = &progress;
        let sst = &sst;
        let counters = &counters;
        let hot_set = &hot_set;
        let scanned = &scanned;

        // Loader: loads and sorts partitions in order, one ahead of the
        // writer.
        let (job_tx, job_rx) = channel::unbounded::<PartitionJob>();
        let (sorted_tx, sorted_rx) = channel::bounded::<(usize, Partition, HotEntries)>(0);
        s.spawn(move || {
            for job in job_rx {
                let memory_partition = job.memory_partition;
                let load_threads = if scanned.load(Ordering::Relaxed) {
                    final_load_threads
                } else {
                    load_threads
                };
                let timer = Instant::now();
                let mut partition = match job.source {
                    PartitionSource::Resident(parts) => {
                        partition::from_parts(parts, layout.memory_partitions(), load_threads)
                    }
                    PartitionSource::Scratch(scratch, records) => {
                        let codec = ScratchCodec::new(memory_partition, layout.memory_partitions());
                        let partition = partition::load(
                            &scratch,
                            records as usize,
                            codec,
                            layout.memory_partitions(),
                            load_threads,
                        );
                        counters.remove_scratch(scratch).expect("remove scratch");
                        partition
                    }
                };
                let load = timer.elapsed();

                let timer = Instant::now();
                partition.sort(load_threads);
                log_above(
                    progress,
                    format!(
                        "partition {memory_partition}: load {load:.1?}, sort {:.1?} ({} records, {} hot entries)",
                        timer.elapsed(),
                        partition.records(),
                        job.hot.len(),
                    ),
                );
                sorted_tx
                    .send((memory_partition, partition, job.hot))
                    .expect("send sorted");
            }
        });

        s.spawn(move || {
            for (memory_partition, partition, hot) in sorted_rx {
                let sst_threads = if scanned.load(Ordering::Relaxed) {
                    final_sst_threads
                } else {
                    sst_threads
                };
                let timer = Instant::now();
                write_partition(
                    &partition,
                    &hot,
                    memory_partition,
                    layout.memory_partitions(),
                    sst,
                    sst_threads,
                );
                drop(partition);
                counters.written_partitions.add(1);
                let stats = sst.stats.lock().expect("lock stats");
                log_above(
                    progress,
                    format!(
                        "partition {memory_partition}: write {:.1?} ({} cold files with {:.1} B/record, {} positions files with {:.1} B/key so far)",
                        timer.elapsed(),
                        stats.cold.files,
                        stats.cold.bytes as f64 / stats.cold_records.max(1) as f64,
                        stats.positions.files,
                        stats.positions.bytes_per_key(),
                    ),
                );
            }
        });

        for pass in 0..layout.passes() {
            progress.set_prefix(format!("pass {}/{}", pass + 1, layout.passes()));
            if !config.overlap && pass > 0 {
                progress.set_message("waiting for previous pass");
                let waited = counters
                    .written_partitions
                    .wait_until(|n| n >= (pass * layout.memory_partitions_per_pass()) as u64);
                log_above(
                    progress,
                    format!("pass {pass}: waited {waited:.1?} for previous pass"),
                );
            }
            progress.set_message("scattering");
            let timer = Instant::now();
            let scattered = scatter_pass(
                input,
                pass,
                &ScatterConfig {
                    scratch: scratch_dir,
                    zstd_level: config.scratch_zstd_level,
                    // The first pass has nothing to wait for.
                    budget: (pass > 0).then_some(estimate.scratch_budget),
                    layout: layout.clone(),
                    read_threads,
                    write_threads,
                    io_depth: config.io_depth.max(1),
                    direct_reads: config.direct_reads,
                    counters,
                    hot_set,
                },
                progress,
            )
            .expect("scatter");
            if pass + 1 == layout.passes() || config.scatter_only {
                scanned.store(true, Ordering::Relaxed);
            }
            let scratch_bytes: u64 = scattered.scratch.iter().map(ScratchFile::len).sum();
            let scratch_raw: u64 = scattered.scratch.iter().map(ScratchFile::raw_len).sum();
            let scratch_records: u64 = scattered.scratch_records.iter().sum();
            counters
                .reclaimable_scratch
                .fetch_add(scratch_bytes, Ordering::Release);
            log_above(
                progress,
                format!(
                    "pass {pass}: scatter {:.1?} ({} MiB scratch, {:.2} B/record, raw {:.2} B/record, {} hot entries)",
                    timer.elapsed(),
                    scratch_bytes >> 20,
                    scratch_bytes as f64 / scratch_records.max(1) as f64,
                    scratch_raw as f64 / scratch_records.max(1) as f64,
                    scattered.hot.len(),
                ),
            );
            if config.scatter_only {
                for file in scattered.scratch {
                    counters.remove_scratch(file).expect("remove scratch");
                }
                break;
            }

            // Hot entries of each memory partition of this pass.
            let first_memory_partition = layout.first_memory_partition(pass);
            let mut hot_by_partition: Vec<HotEntries> = (0..layout.memory_partitions_per_pass())
                .map(|_| Vec::new())
                .collect();
            for (key, value) in scattered.hot {
                let entry_partition = memory_partition(&key, layout.memory_partitions());
                hot_by_partition[entry_partition - first_memory_partition].push((key, value));
            }

            // Process the resident partitions first, to free their memory.
            let scratch_partitions = layout.scratch_partitions_per_pass();
            for (i, parts) in scattered.resident.into_iter().enumerate() {
                let j = scratch_partitions + i;
                job_tx
                    .send(PartitionJob {
                        memory_partition: first_memory_partition + j,
                        source: PartitionSource::Resident(parts),
                        hot: mem::take(&mut hot_by_partition[j]),
                    })
                    .expect("send job");
            }
            for (j, (file, records)) in scattered
                .scratch
                .into_iter()
                .zip(scattered.scratch_records)
                .enumerate()
            {
                job_tx
                    .send(PartitionJob {
                        memory_partition: first_memory_partition + j,
                        source: PartitionSource::Scratch(file, records),
                        hot: mem::take(&mut hot_by_partition[j]),
                    })
                    .expect("send job");
            }
        }

        drop(job_tx);
    });

    progress.finish();

    let records = counters.records.load(Ordering::Relaxed);
    let hot_records = counters.hot_records.load(Ordering::Relaxed);
    Ok(Stats {
        games: counters.games.load(Ordering::Relaxed),
        records: records + hot_records,
        hot_positions: hot_set.len(),
        hot_records,
        output: sst.stats.into_inner().expect("stats"),
    })
}

struct ScatterConfig<'a> {
    scratch: &'a Path,
    zstd_level: i32,
    /// Scratch budget to throttle against, if the previous pass is still
    /// freeing scratch space.
    budget: Option<u64>,
    layout: Layout,
    read_threads: usize,
    write_threads: usize,
    io_depth: usize,
    direct_reads: bool,
    counters: &'a Counters,
    hot_set: &'a IntSet<PositionKey>,
}

/// Accumulated time of each scatter stage, summed over its threads.
#[derive(Default)]
struct ScatterTimes {
    prefetch: PrefetchTimes,
    recv: AtomicU64,
    decode: AtomicU64,
    replay: AtomicU64,
    send: AtomicU64,
    writer: ChunkWriterTimes,
}

impl ScatterTimes {
    fn summary(&self) -> String {
        format!(
            "prefetch: fetch {:.0}s, send {:.0}s; read: recv {:.0}s, decode {:.0}s, replay {:.0}s, send {:.0}s; compress: {}",
            timed_seconds(&self.prefetch.fetch),
            timed_seconds(&self.prefetch.send),
            timed_seconds(&self.recv),
            timed_seconds(&self.decode),
            timed_seconds(&self.replay),
            timed_seconds(&self.send),
            self.writer.summary(),
        )
    }
}

struct Scattered {
    /// Parts of the resident partitions collected by each thread.
    resident: Vec<Vec<Vec<IntermediateEntry>>>,
    scratch: Vec<ScratchFile>,
    /// Records in each scratch file.
    scratch_records: Vec<u64>,
    /// Packed entries of the hot positions of this pass.
    hot: HotEntries,
}

/// Replays the entire input and distributes the records of the given disk
/// partition to its memory partitions. The last `RESIDENT_PARTITIONS_PER_PASS`
/// memory partitions are kept in memory, the others are written to scratch
/// files. Records of hot positions are aggregated in memory.
fn scatter_pass(
    input: &Input,
    pass: usize,
    config: &ScatterConfig<'_>,
    progress: &ProgressBar,
) -> io::Result<Scattered> {
    let scratch_partitions = config.layout.scratch_partitions_per_pass();
    let first_memory_partition = config.layout.first_memory_partition(pass);
    let counters = config.counters;

    let files: Vec<Mutex<ScratchFile>> = (0..scratch_partitions)
        .map(|i| {
            Ok::<_, io::Error>(Mutex::new(ScratchFile::create(
                config.scratch.join(format!(
                    "positions_{:05}.chunks",
                    first_memory_partition + i
                )),
                config.zstd_level,
                config.direct_reads,
            )?))
        })
        .collect::<io::Result<_>>()?;

    let scratch_records: Vec<AtomicU64> =
        (0..scratch_partitions).map(|_| AtomicU64::new(0)).collect();
    let codecs: Vec<ScratchCodec> = (0..scratch_partitions)
        .map(|j| {
            ScratchCodec::new(
                first_memory_partition + j,
                config.layout.memory_partitions(),
            )
        })
        .collect();
    let resident: Mutex<Vec<Vec<Vec<IntermediateEntry>>>> = Mutex::new(Vec::new());
    let hot_map = HotMap::default();
    let times = ScatterTimes::default();
    let writer_config = ChunkWriterConfig {
        threads: config.write_threads,
        zstd_level: config.zstd_level,
        budget: config.budget,
        usage: &counters.scratch_usage,
        reclaimable: &counters.reclaimable_scratch,
    };

    let encode = &|j: usize, chunk: Chunk, out: &mut Vec<u8>| {
        scratch_records[j].fetch_add(chunk.len() as u64, Ordering::Relaxed);
        codecs[j].encode(&chunk, out);
    };

    thread::scope(|s| {
        let files = &files;
        let resident = &resident;
        let hot_map = &hot_map;
        let times = &times;
        let writer_config = &writer_config;
        let prefetcher = Prefetcher::spawn(input, config.io_depth, &times.prefetch, s);

        // Write threads encode and compress full chunks and append them to
        // scratch files, throttled by the scratch budget.
        let (tx, rx) = channel::bounded::<(usize, Chunk)>(config.write_threads * 2);
        spawn_chunk_writers(s, writer_config, files, rx, encode, &times.writer);

        // Read threads replay the games and route the records of this pass
        // to hot positions, resident partitions or scratch chunks.
        for _ in 0..config.read_threads {
            let tx = tx.clone();
            let prefetcher = prefetcher.clone();
            s.spawn(move || {
                let mut replayer = Replayer::new();
                let mut hot = HotBuffer::new(hot_map);
                let mut local_resident: Vec<Vec<IntermediateEntry>> = (0
                    ..RESIDENT_PARTITIONS_PER_PASS)
                    .map(|_| Vec::new())
                    .collect();
                let mut chunks: Vec<Chunk> = (0..scratch_partitions).map(|_| new_chunk()).collect();
                let mut games = 0;
                let (mut records, mut hot_records) = (0, 0);

                while let Some(fetched) = timed(&times.recv, || prefetcher.recv()) {
                    let first_row = input.first_row(fetched.0);
                    let batch = timed(&times.decode, || prefetcher.decode(fetched))
                        .expect("read row group");
                    // Time spent blocked on the chunk writers is not replay.
                    let timer = Instant::now();
                    let mut sending = Duration::ZERO;
                    for (k, (id, game)) in batch.rows().enumerate() {
                        let index = GameIndex::new(first_row + k as u64).expect("game index");
                        games += 1;
                        let info = match replayer.replay_game(&game, index) {
                            Ok(info) => info,
                            Err(err) => {
                                if pass == 0 {
                                    progress.println(format!("skipping game {id}: {err}"));
                                }
                                continue;
                            }
                        };

                        for entry in replayer.entries(&info) {
                            let Some(j) = config.layout.pass_memory_partition(
                                memory_partition(entry.key(), config.layout.memory_partitions()),
                                pass,
                            ) else {
                                continue;
                            };
                            if config.hot_set.contains(&entry.hash()) {
                                hot.push(entry);
                                hot_records += 1;
                            } else if j >= scratch_partitions {
                                local_resident[j - scratch_partitions].push(entry);
                                records += 1;
                            } else {
                                let chunk = &mut chunks[j];
                                chunk.push(entry.0);
                                records += 1;
                                if chunk.len() >= CHUNK_RECORDS {
                                    let full = mem::replace(chunk, new_chunk());
                                    let send_timer = Instant::now();
                                    tx.send((j, full)).expect("send chunk");
                                    sending += send_timer.elapsed();
                                }
                            }
                        }
                    }
                    times
                        .send
                        .fetch_add(sending.as_nanos() as u64, Ordering::Relaxed);
                    times.replay.fetch_add(
                        (timer.elapsed() - sending).as_nanos() as u64,
                        Ordering::Relaxed,
                    );

                    progress.inc(batch.len() as u64);
                }

                for (j, chunk) in chunks.into_iter().enumerate() {
                    if !chunk.is_empty() {
                        tx.send((j, chunk)).expect("send chunk");
                    }
                }

                if pass == 0 {
                    counters.games.fetch_add(games, Ordering::Relaxed);
                }
                counters.records.fetch_add(records, Ordering::Relaxed);
                counters
                    .hot_records
                    .fetch_add(hot_records, Ordering::Relaxed);
                resident.lock().expect("lock resident").push(local_resident);
            });
        }

        // Write threads finish once all chunks are written.
        drop(tx);
    });

    log_above(progress, format!("pass {pass}: {}", times.summary()));

    let files: Vec<ScratchFile> = files
        .into_iter()
        .map(|file| file.into_inner().expect("scratch file"))
        .collect();

    // Group the per-thread parts by resident partition.
    let mut merged: Vec<Vec<Vec<IntermediateEntry>>> = (0..RESIDENT_PARTITIONS_PER_PASS)
        .map(|_| Vec::new())
        .collect();
    for local in resident.into_inner().expect("resident") {
        for (i, part) in local.into_iter().enumerate() {
            merged[i].push(part);
        }
    }

    Ok(Scattered {
        resident: merged,
        scratch: files,
        scratch_records: scratch_records
            .iter()
            .map(|records| records.load(Ordering::Relaxed))
            .collect(),
        hot: hot_map.pack(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cold_zstd_level() {
        // Masters (all plies), and lichess (50 plies).
        assert_eq!(cold_zstd_level(240_857_745), 3);
        assert_eq!(cold_zstd_level(300_000_000_000), 0);
        assert_eq!(cold_zstd_level((1 << 32) - 1), 3);
        assert_eq!(cold_zstd_level(1 << 32), 0);
    }

    #[test]
    fn test_output_bound() {
        // Records and SST bytes of real runs: masters, lichess slices of 60
        // and 1000 row groups, and all of lichess.
        for (records, bytes) in [
            (240_857_745, 2_311_965_762),
            (451_302_685, 3_759_064_289),
            (4_404_320_877, 51_446_524_315),
            (378_968_287_958, 2_157_316_170_701),
        ] {
            let bound = output_bound(records);
            assert!(bytes < bound && bound < 3 * bytes, "{records}: {bound}");
        }
    }

    #[test]
    fn test_scratch_budget() {
        let free = |shared_with_output| ScratchBudget::Free {
            bytes: 1000,
            shared_with_output,
        };
        assert_eq!(ScratchBudget::Bytes(1000).resolve(5000).unwrap(), 1000);
        assert_eq!(free(false).resolve(5000).unwrap(), 900);
        assert_eq!(free(true).resolve(400).unwrap(), 540);
        assert!(free(true).resolve(1001).is_err());
    }
}
