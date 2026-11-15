mod scratch;

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use crossbeam::channel;
pub use scratch::{ChunkCompressor, ScratchFile};

use crate::util::{Gauge, timed, timed_seconds};

/// Memory partitions per pass that are kept in memory while reading the
/// input, rather than being written to scratch space and read back.
pub const RESIDENT_PARTITIONS_PER_PASS: usize = 1;

/// How the key space is split: `passes` passes over the input, each
/// covering `memory_partitions_per_pass` memory partitions. Keys are mapped
/// to memory partitions by multiply-shift, so neither count needs to be a
/// power of two.
#[derive(Debug, Clone)]
pub struct Layout {
    passes: usize,
    memory_partitions_per_pass: usize,
}

impl Layout {
    /// Upper bound for the memory partitions of a layout.
    const MAX_MEMORY_PARTITIONS: usize = 1 << 16;

    pub fn new(passes: usize, memory_partitions_per_pass: usize) -> Layout {
        assert!(
            passes >= 1 && memory_partitions_per_pass >= 1,
            "empty layout"
        );
        assert!(
            passes * memory_partitions_per_pass <= Self::MAX_MEMORY_PARTITIONS,
            "at most {} memory partitions",
            Self::MAX_MEMORY_PARTITIONS
        );
        Layout {
            passes,
            memory_partitions_per_pass,
        }
    }

    pub fn passes(&self) -> usize {
        self.passes
    }

    pub fn memory_partitions(&self) -> usize {
        self.passes * self.memory_partitions_per_pass
    }

    pub fn memory_partitions_per_pass(&self) -> usize {
        self.memory_partitions_per_pass
    }

    pub fn scratch_partitions_per_pass(&self) -> usize {
        self.memory_partitions_per_pass - RESIDENT_PARTITIONS_PER_PASS
    }

    /// The first memory partition of a pass.
    pub fn first_memory_partition(&self, pass: usize) -> usize {
        pass * self.memory_partitions_per_pass
    }

    /// Index of the given memory partition within the given pass, or `None`
    /// if it belongs to a different pass.
    #[inline]
    pub fn pass_memory_partition(&self, memory_partition: usize, pass: usize) -> Option<usize> {
        memory_partition
            .checked_sub(self.first_memory_partition(pass))
            .filter(|j| *j < self.memory_partitions_per_pass)
    }

    /// Picks the fewest passes that fit the scratch budget, and the fewest
    /// memory partitions per pass that fit the memory budget. `fits` returns
    /// whether a layout fits the memory and the scratch budget, respectively.
    pub fn choose(fits: impl Fn(&Layout) -> (bool, bool)) -> Layout {
        for passes in 1..=Self::MAX_MEMORY_PARTITIONS {
            for memory_partitions_per_pass in 1..=Self::MAX_MEMORY_PARTITIONS / passes {
                let layout = Layout {
                    passes,
                    memory_partitions_per_pass,
                };
                match fits(&layout) {
                    (true, true) => return layout,
                    // More partitions per pass only add scratch. Try more
                    // passes instead.
                    (true, false) => break,
                    (false, _) => (),
                }
            }
        }
        panic!("no layout fits the budgets");
    }
}

/// Accumulated time of the chunk writer threads.
#[derive(Default)]
pub struct ChunkWriterTimes {
    pub recv: AtomicU64,
    pub encode: AtomicU64,
    pub compress: AtomicU64,
    pub write: AtomicU64,
    pub throttled: AtomicU64,
}

impl ChunkWriterTimes {
    pub fn summary(&self) -> String {
        format!(
            "recv {:.0}s, encode {:.0}s, compress {:.0}s, write {:.0}s, throttled {:.0}s",
            timed_seconds(&self.recv),
            timed_seconds(&self.encode),
            timed_seconds(&self.compress),
            timed_seconds(&self.write),
            timed_seconds(&self.throttled),
        )
    }
}

pub struct ChunkWriterConfig<'a> {
    pub threads: usize,
    pub zstd_level: i32,
    /// Scratch budget to throttle against, if a previous pass is still
    /// freeing scratch space.
    pub budget: Option<u64>,
    /// Bytes currently occupied by scratch files.
    pub usage: &'a Gauge,
    /// Bytes of scratch files of previous passes that are still to be
    /// loaded and removed. Once nothing is left to reclaim, waiting for the
    /// budget could never succeed, so the writers continue and exceed the
    /// budget by the error of the scratch estimate instead of deadlocking.
    pub reclaimable: &'a AtomicU64,
}

/// Spawns threads that encode chunks received as `(file, chunk)` with
/// `encode`, compress them and append them to the scratch files, throttled
/// by the scratch budget. They finish once all senders are dropped.
pub fn spawn_chunk_writers<'s, C: Send + 's>(
    s: &'s thread::Scope<'s, '_>,
    config: &'s ChunkWriterConfig<'s>,
    files: &'s [Mutex<ScratchFile>],
    rx: channel::Receiver<(usize, C)>,
    encode: &'s (impl Fn(usize, C, &mut Vec<u8>) + Sync),
    times: &'s ChunkWriterTimes,
) {
    for _ in 0..config.threads {
        let rx = rx.clone();
        s.spawn(move || {
            let mut compressor = ChunkCompressor::new(config.zstd_level);
            let mut raw = Vec::new();
            while let Ok((file, chunk)) = timed(&times.recv, || rx.recv()) {
                timed(&times.encode, || encode(file, chunk, &mut raw));
                let compressed = timed(&times.compress, || compressor.compress(&raw));
                if let Some(budget) = config.budget {
                    let waited = config.usage.wait_until(|usage| {
                        usage + compressed.len() as u64 <= budget
                            || config.reclaimable.load(Ordering::Acquire) == 0
                    });
                    times
                        .throttled
                        .fetch_add(waited.as_nanos() as u64, Ordering::Relaxed);
                }
                timed(&times.write, || {
                    files[file].lock().expect("lock scratch file").append(
                        raw.len(),
                        compressed,
                        config.usage,
                    )
                })
                .expect("append chunk");
            }
        });
    }
}
