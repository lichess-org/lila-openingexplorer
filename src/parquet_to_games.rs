use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use indicatif::{MultiProgress, ProgressBar};

use crate::{
    packed::{GameId, GameIndex, pack_game_value},
    parquet::Input,
    sst::{RollingSstWriter, SstConfig, SstError, SstStats, env_options},
    util::{create_empty_dir, log_above, progress_bar},
};

/// Estimated SST bytes per game (key, id and packed game), to split the
/// input into ranges of about one file each.
const ESTIMATED_BYTES_PER_GAME: f64 = 95.0;

/// Hard limit for SST files, as a multiple of the target size, in case the
/// size estimate of a range is far off.
const MAX_FILE_SIZE_FACTOR: u64 = 2;

pub struct Config {
    /// Threads writing ranges of the input in parallel.
    pub threads: usize,
    /// Use O_DIRECT to read the input.
    pub direct_reads: bool,
    /// Use O_DIRECT to write the SST files.
    pub direct_writes: bool,
    /// Skip the first row groups of the inputs.
    pub skip_row_groups: usize,
    /// Only process the first row groups of the inputs.
    pub row_groups: Option<usize>,
    /// Approximate size of the SST files.
    pub sst_target_size: u64,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub games: SstStats,
    pub game_ids: Option<SstStats>,
}

/// Writes the games of the parquet files to non-overlapping SST files in
/// `out`, keyed by `GameIndex`. The input is already in key order, so
/// contiguous ranges of row groups are written in parallel, each to its
/// own files. With `game_ids`, also writes the map from `GameId` to
/// `GameIndex` there (kept in memory until the end, so only for small
/// inputs, like the masters database).
pub fn convert(
    parquet: &[PathBuf],
    out: &Path,
    game_ids: Option<&Path>,
    config: &Config,
    progress: &MultiProgress,
) -> Result<Stats, SstError> {
    let input = Input::open(
        parquet,
        config.skip_row_groups,
        config.row_groups,
        config.direct_reads,
    )?;
    create_empty_dir(out)?;
    if let Some(dir) = game_ids {
        create_empty_dir(dir)?;
    }

    let opts = SstConfig::GAMES.options();
    let env_opts = env_options(config.direct_writes);
    let ranges = ranges(&input, config);
    let progress = progress.add(progress_bar(input.total_rows()));
    let next = AtomicUsize::new(0);
    let stats = Mutex::new(SstStats::default());
    let ids: Mutex<Vec<(GameId, GameIndex)>> = Mutex::new(Vec::new());

    thread::scope(|s| -> Result<(), SstError> {
        let handles: Vec<_> = (0..config.threads.clamp(1, ranges.len().max(1)))
            .map(|_| {
                s.spawn(|| -> Result<(), SstError> {
                    while let Some(range) = ranges.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let started = Instant::now();
                        let (written, range_ids) = write_range(
                            &input,
                            range.clone(),
                            &opts,
                            &env_opts,
                            &out.join(format!("{:06}", range.start)),
                            config.sst_target_size,
                            game_ids.is_some(),
                            &progress,
                        )?;
                        log_above(
                            &progress,
                            format!(
                                "row groups {}..{}: {} games in {} files in {:.1?}",
                                range.start,
                                range.end,
                                written.keys,
                                written.files,
                                started.elapsed()
                            ),
                        );
                        stats.lock().expect("stats").merge(&written);
                        ids.lock().expect("ids").extend(range_ids);
                    }
                    Ok(())
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("range writer")?;
        }
        Ok(())
    })?;
    progress.finish();

    let game_ids = game_ids
        .map(|dir| write_game_ids(ids.into_inner().expect("ids"), dir, config))
        .transpose()?;
    Ok(Stats {
        games: stats.into_inner().expect("stats"),
        game_ids,
    })
}

/// Contiguous ranges of row groups, of about one target file size each, but
/// at least one per thread.
fn ranges(input: &Input, config: &Config) -> Vec<std::ops::Range<usize>> {
    let row_groups = input.num_row_groups();
    let estimated = input.total_rows() as f64 * ESTIMATED_BYTES_PER_GAME;
    let count = ((estimated / config.sst_target_size as f64).ceil() as usize)
        .max(config.threads)
        .clamp(1, row_groups.max(1));
    let rows = input.total_rows();
    // Split by rows, at row group boundaries.
    let mut ranges = Vec::with_capacity(count);
    let mut start = 0;
    let mut seen = 0;
    for i in 0..row_groups {
        seen += input.num_rows(i);
        if seen * count as u64 >= rows * (ranges.len() as u64 + 1) {
            ranges.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < row_groups {
        ranges.push(start..row_groups);
    }
    ranges
}

#[allow(clippy::too_many_arguments)]
fn write_range(
    input: &Input,
    range: std::ops::Range<usize>,
    opts: &rocksdb::Options,
    env_opts: &rocksdb::EnvOptions,
    prefix: &Path,
    target_size: u64,
    collect_ids: bool,
    progress: &ProgressBar,
) -> Result<(SstStats, Vec<(GameId, GameIndex)>), SstError> {
    let mut writer = RollingSstWriter::new(
        opts,
        env_opts,
        prefix.into(),
        MAX_FILE_SIZE_FACTOR * target_size,
    );
    let mut ids = Vec::new();
    let mut value = Vec::new();
    for row_group in range {
        let batch = input
            .fetch(row_group)
            .and_then(|bytes| input.read_games_from(row_group, bytes))
            .map_err(io::Error::other)?;
        let first_row = input.first_row(row_group);
        for (k, (id, game)) in batch.rows().enumerate() {
            let index = GameIndex::new(first_row + k as u64).expect("game index");
            value.clear();
            pack_game_value(id, &game, &mut value);
            writer.put(&index.to_bytes(), &value)?;
            if collect_ids {
                ids.push((id, index));
            }
        }
        progress.inc(input.num_rows(row_group));
    }
    Ok((writer.finish()?, ids))
}

fn write_game_ids(
    mut ids: Vec<(GameId, GameIndex)>,
    dir: &Path,
    config: &Config,
) -> Result<SstStats, SstError> {
    ids.sort_unstable();
    if let Some(pair) = ids.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(io::Error::other(format!("duplicate game id {}", pair[0].0)).into());
    }
    let opts = SstConfig::GAME_IDS.options();
    let env_opts = env_options(config.direct_writes);
    let mut writer = RollingSstWriter::new(
        &opts,
        &env_opts,
        dir.join("game_ids"),
        config.sst_target_size,
    );
    for (id, index) in ids {
        writer.put(&id.to_bytes(), &index.to_bytes())?;
    }
    writer.finish()
}
