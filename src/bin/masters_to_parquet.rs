use std::{path::PathBuf, process, thread};

use clap::Parser;
use lila_openingexplorer4::{
    masters_to_parquet::{Config, collect_pgns, convert, read_legacy_ids},
    util::zstd_level,
};
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

/// Converts the masters database (PGN files, or directories like `games/` to
/// walk recursively, in sorted order) to a single parquet file with the same
/// schema as produced by pgn_to_parquet, plus event, site and round columns.
///
/// Game ids are deterministic hashes of the game content, compatible with
/// the existing masters database, replaced by legacy ids where the first
/// version of the database assigned one. Games with fewer than two plies,
/// games that cannot be parsed, and exact duplicates are skipped.
#[derive(Parser)]
struct Args {
    #[clap(required = true)]
    paths: Vec<PathBuf>,
    /// Output parquet file.
    #[clap(long)]
    out: PathBuf,
    /// The legacy_ids.csv of the masters database.
    #[clap(long)]
    legacy_ids: PathBuf,
    /// Threads to use. Defaults to the available parallelism minus two.
    #[clap(long)]
    threads: Option<usize>,
    /// Games per row group.
    #[clap(long, default_value_t = 100000)]
    row_group_len: usize,
    /// Zstd level for meta data columns in the produced parquet file, or 0
    /// for no compression.
    #[clap(long, default_value_t = 9, value_parser = zstd_level())]
    zstd_level: i32,
}

fn main() {
    let args = Args::parse();

    let config = Config {
        threads: args.threads.unwrap_or_else(|| {
            thread::available_parallelism()
                .map_or(1, |n| n.get())
                .saturating_sub(2)
                .max(1)
        }),
        row_group_len: args.row_group_len,
        zstd_level: args.zstd_level,
    };

    let result = read_legacy_ids(&args.legacy_ids).and_then(|legacy_ids| {
        let pgns = collect_pgns(&args.paths)?;
        convert(&pgns, &legacy_ids, &args.out, &config)
    });
    match result {
        Ok(stats) => println!(
            "{} games ({} with legacy ids), {} unplayed skipped, {} duplicates skipped, {} invalid skipped",
            stats.games, stats.legacy, stats.unplayed, stats.duplicates, stats.invalid
        ),
        Err(err) => {
            eprintln!("error: {err}");
            process::exit(1);
        }
    }
}
