//! Offline build of a prod-like personal explorer database, for benchmarks
//! (milestone 4 of `player-explorer.md`):
//!
//! 1. `population`: game counts of all players, and a selection of players
//!    (the `requests.tsv` players and a sample of others).
//! 2. `scatter`: one pass over the parquet that writes the games of the
//!    selected players, as records of the `games` column family, to
//!    partitions by user hash.
//! 3. `build`: per partition, the games up to a cutoff per player as
//!    sorted, non-overlapping SST files of all column families (the same
//!    aggregation and placement as a rebuild), to ingest into the bottom
//!    level. The games after the cutoffs that are to be indexed by the
//!    aging step go to tail files, by the time of the visit that indexes
//!    them.
//! 4. `age`: the tails through the live write path (`write_chunk`,
//!    rebuilds), interleaved in time order across players.
//!
//! Records are synthesized with the rules of the mock lila
//! ([`crate::mock_lila::synth`]), so that the database agrees with what the
//! mock serves for the same players.

pub mod age;
pub mod build;
pub mod check;
pub mod population;
pub mod scatter;

/// File with the key of the user hashes (`UserHasher`) that `scatter`
/// generates, which `build` copies to its output for `ingest`.
pub const USER_HASH_KEY_FILE: &str = "user_hash_key";

/// Writes the key of a user hasher to a file.
pub fn save_user_hasher(hasher: &UserHasher, path: &Path) -> io::Result<()> {
    fs::write(path, hasher.key())
}

/// Reads a key file written by `save_user_hasher`.
pub fn load_user_hasher(path: &Path) -> io::Result<UserHasher> {
    UserHasher::from_key_bytes(&fs::read(path)?).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid user hash key in {}", path.display()),
        )
    })
}

use std::{
    fs, io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use crate::{
    parquet::{GameBatch, Input},
    player::UserHasher,
};

/// Runs `f` on all row groups of the input, on `threads` threads, each with
/// its own state from `init`. Returns the states.
pub fn for_each_row_group<S: Send>(
    input: &Input,
    threads: usize,
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, usize, &GameBatch) + Sync,
) -> io::Result<Vec<S>> {
    let n = input.num_row_groups();
    let next = AtomicUsize::new(0);
    let rows = AtomicU64::new(0);
    let started = Instant::now();
    let error: Mutex<Option<io::Error>> = Mutex::new(None);
    let states = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut state = init();
                    loop {
                        let rg = next.fetch_add(1, Ordering::Relaxed);
                        if rg >= n || error.lock().expect("error").is_some() {
                            break;
                        }
                        let batch = match input.read_games(rg) {
                            Ok(batch) => batch,
                            Err(err) => {
                                *error.lock().expect("error") = Some(io::Error::other(err));
                                break;
                            }
                        };
                        f(&mut state, rg, &batch);
                        let done = rows.fetch_add(batch.len() as u64, Ordering::Relaxed)
                            + batch.len() as u64;
                        if rg.is_multiple_of(1000) {
                            log::info!(
                                "row group {rg}/{n}, {done} rows, {:.0} s",
                                started.elapsed().as_secs_f64()
                            );
                        }
                    }
                    state
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("row group thread"))
            .collect::<Vec<S>>()
    });
    if let Some(err) = error.into_inner().expect("error") {
        return Err(err);
    }
    log::info!(
        "{} rows in {:.0} s",
        rows.load(Ordering::Relaxed),
        started.elapsed().as_secs_f64()
    );
    Ok(states)
}
