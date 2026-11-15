use std::{path::PathBuf, process, thread};

use clap::Parser;
use indicatif::{MultiProgress, ProgressDrawTarget};
use lila_openingexplorer4::{
    pgn_to_parquet::{Config, convert},
    util::{ByteSize, total_memory, zstd_level},
};
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

/// Converts PGN files (optionally zstd compressed) to a single parquet file.
///
/// Pass the files of each variant in chronological order (like the monthly
/// lichess database dumps): rows are game indexes, which should be
/// chronological by month within each variant.
///
/// Unplayed games (fewer than two plies), unfinished games (without a known
/// outcome), games of bot accounts (`WhiteTitle` or `BlackTitle` is `BOT`),
/// games where a player has no rating, and games that cannot be parsed are
/// skipped.
///
/// Only the first --max-ply moves of each game are processed.
///
/// Games whose id already appeared among the recent games of the same
/// variant (by `Variant` tag, at least the previous 100k, also across
/// files, like consecutive months) are skipped as duplicates.
#[derive(Parser)]
struct Args {
    pgns: Vec<PathBuf>,
    /// Output parquet file.
    #[clap(long)]
    out: PathBuf,
    /// Threads to use in total. Defaults to the available parallelism minus
    /// two.
    #[clap(long)]
    threads: Option<usize>,
    /// Files read concurrently. Each takes up to one thread. Consider
    /// matching the number of hard drives, or more for NVMe.
    #[clap(long, default_value_t = 4)]
    readers: usize,
    /// Maximum bytes of PGN chunks and encoded row groups held in memory
    /// while waiting for earlier chunks. Chunks of the oldest unfinished
    /// file are exempt, so that progress is always possible. Defaults to
    /// 60% of the total memory. Accepts units, like 32g.
    #[clap(long)]
    memory_budget: Option<ByteSize>,
    /// Games per row group.
    #[clap(long, default_value_t = 100000)]
    row_group_len: usize,
    /// Bytes read from a file at once. Large sequential reads are kinder to
    /// spinning disks shared between readers.
    #[clap(long, default_value = "16m")]
    read_buffer: ByteSize,
    /// Zstd level for meta data columns in the produced parquet file, or 0
    /// for no compression. Gains for levels above 9 are small.
    #[clap(long, default_value_t = 9, value_parser = zstd_level())]
    zstd_level: i32,
    /// Zstd level for the moves column in the produced parquet file, or 0 for
    /// no compression. Stacking compression on the move encoding is not very
    /// effective (~5%).
    #[clap(long, default_value_t = 0, value_parser = zstd_level())]
    moves_zstd_level: i32,
    /// Keep only the first moves of each game, or 0 for all moves. The
    /// lichess explorer indexes the first 50 plies.
    #[clap(long, alias = "max-plies", default_value_t = 50)]
    max_ply: usize,
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
        readers: args.readers,
        games_per_chunk: args.row_group_len,
        memory_budget: args.memory_budget.map_or_else(
            || total_memory().expect("total memory; pass --memory-budget") / 10 * 6,
            u64::from,
        ),
        read_buffer: usize::try_from(args.read_buffer.0).expect("read buffer fits in memory"),
        zstd_level: args.zstd_level,
        moves_zstd_level: args.moves_zstd_level,
        max_ply: (args.max_ply > 0).then_some(args.max_ply),
    };

    let progress = MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(4));
    let stats = match convert(&args.pgns, &args.out, &config, &progress) {
        Ok(stats) => stats,
        Err(err) => {
            eprintln!("error: {err}");
            process::exit(1);
        }
    };

    println!(
        "{} games, {} unplayed skipped, {} unfinished skipped, {} bots skipped, {} without ratings skipped, {} duplicates skipped, {} invalid skipped",
        stats.games,
        stats.unplayed,
        stats.unfinished,
        stats.bots,
        stats.unrated_players,
        stats.duplicates,
        stats.invalid
    );
}
