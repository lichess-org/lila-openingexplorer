use std::{num::NonZeroUsize, path::PathBuf, process, thread, time::Instant};

use clap::Parser;
use indicatif::{MultiProgress, ProgressDrawTarget};
use lila_openingexplorer4::{
    parquet_to_games::{Config, convert},
    util::ByteSize,
};
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

/// Writes the games of parquet files to non-overlapping SST files, keyed by
/// their row in the input (`GameIndex`), which is how positions refer to
/// them.
///
/// The input is already in key order, so ranges of row groups are written
/// in parallel, without sorting.
#[derive(Parser)]
struct Args {
    #[clap(required = true)]
    parquet: Vec<PathBuf>,
    /// Directory for the SST files (which must be empty or missing).
    #[clap(long)]
    out: PathBuf,
    /// Also write the map from game ids to game indexes, for looking up
    /// games by id, to SST files in this directory (which must be empty or
    /// missing). Keeps the map in memory, so only for small inputs, like
    /// the masters database.
    #[clap(long)]
    game_ids: Option<PathBuf>,
    /// Threads to use. Defaults to the available parallelism minus two, to
    /// leave one physical core idle.
    #[clap(long)]
    threads: Option<usize>,
    /// Disable O_DIRECT for reading the input.
    #[clap(long)]
    no_direct_io: bool,
    /// Skip the first row groups of the inputs. Game indexes still count
    /// the skipped rows.
    #[clap(long, default_value_t = 0, help_heading = "Benchmarking")]
    skip_row_groups: usize,
    /// Only process the first row groups of the inputs.
    #[clap(long, help_heading = "Benchmarking")]
    row_groups: Option<usize>,
    /// Approximate size of the SST files. The target_file_size_base of the
    /// column family should match.
    #[clap(long, default_value = "2g")]
    sst_target_size: ByteSize,
}

fn main() {
    let args = Args::parse();

    let config = Config {
        threads: args.threads.unwrap_or_else(|| {
            thread::available_parallelism()
                .map_or(1, NonZeroUsize::get)
                .saturating_sub(2)
                .max(2)
        }),
        direct_reads: !args.no_direct_io,
        skip_row_groups: args.skip_row_groups,
        row_groups: args.row_groups,
        sst_target_size: args.sst_target_size.into(),
    };

    let started = Instant::now();
    let progress = MultiProgress::with_draw_target(ProgressDrawTarget::stdout_with_hz(4));
    let stats = match convert(
        &args.parquet,
        &args.out,
        args.game_ids.as_deref(),
        &config,
        &progress,
    ) {
        Ok(stats) => stats,
        Err(err) => {
            eprintln!("error: {err}");
            process::exit(1);
        }
    };

    let games = &stats.games;
    println!(
        "done in {:.1?}: {} games in {} sst files ({} bytes, largest {} bytes, ratio {:.3})",
        started.elapsed(),
        games.keys,
        games.files,
        games.bytes,
        games.largest_file,
        games.ratio(),
    );
    if let Some(ids) = &stats.game_ids {
        println!(
            "game ids: {} ids in {} sst files ({} bytes)",
            ids.keys, ids.files, ids.bytes
        );
    }
}
