// Temporary tool: removes games of bot accounts and games where a player has
// no rating from a parquet file of an older pgn_to_parquet, as the current
// pgn_to_parquet would have written it.
// Game indexes change, so games and positions have to be rebuilt.

use std::{
    collections::BTreeMap,
    fs::File,
    io::BufWriter,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use bytes::Bytes;
use crossbeam::channel;
use lila_openingexplorer4::{
    parquet::{GameColumns, Input},
    pgn_to_parquet::{append_row_group, writer_properties},
    util::progress_bar,
};
use parquet::file::writer::SerializedFileWriter;
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

fn main() {
    let usage = "usage: parquet_filter <in.parquet> <out.parquet> [threads=16] [zstd_level=9] [moves_zstd_level=0] [skip_row_groups=0] [row_groups]";
    let mut args = std::env::args().skip(1);
    let input_path = PathBuf::from(args.next().expect(usage));
    let out_path = PathBuf::from(args.next().expect(usage));
    let threads: usize = args.next().map_or(16, |s| s.parse().expect("threads"));
    let zstd_level: i32 = args.next().map_or(9, |s| s.parse().expect("zstd_level"));
    let moves_zstd_level: i32 = args
        .next()
        .map_or(0, |s| s.parse().expect("moves_zstd_level"));
    let skip: usize = args
        .next()
        .map_or(0, |s| s.parse().expect("skip_row_groups"));
    let limit: Option<usize> = args.next().map(|s| s.parse().expect("row_groups"));
    assert_ne!(input_path, out_path);

    let input =
        Input::open(std::slice::from_ref(&input_path), skip, limit, true).expect("open input");
    let properties = writer_properties(zstd_level, moves_zstd_level);
    let mut writer = SerializedFileWriter::new(
        BufWriter::with_capacity(4 << 20, File::create(&out_path).expect("create output")),
        GameColumns::schema(),
        Arc::clone(&properties),
    )
    .expect("writer");

    let started = Instant::now();
    let num_row_groups = input.num_row_groups();
    let progress = progress_bar(input.total_rows());
    let next = AtomicUsize::new(0);
    let window = 4 * threads.max(1);
    let (token_tx, token_rx) = channel::bounded::<()>(window);
    for _ in 0..window {
        token_tx.send(()).expect("token");
    }
    let (tx, rx) = channel::unbounded::<(usize, Bytes, usize, u64, u64)>();

    let (games, bots, unrated_players, empty_row_groups) = thread::scope(|s| {
        for _ in 0..threads.max(1) {
            let tx = tx.clone();
            let token_rx = token_rx.clone();
            let (input, properties, next) = (&input, &properties, &next);
            s.spawn(move || {
                let mut columns = GameColumns::default();
                while token_rx.recv().is_ok() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= num_row_groups {
                        break;
                    }
                    let batch = input
                        .fetch(i)
                        .and_then(|bytes| input.read_games_from(i, bytes))
                        .expect("read row group");
                    columns.clear();
                    columns.reserve(batch.len());
                    let (mut bots, mut unrated_players) = (0u64, 0u64);
                    for (id, game) in batch.rows() {
                        if game.has_bot() {
                            bots += 1;
                        } else if game.players.iter().any(|player| player.rating.is_none()) {
                            unrated_players += 1;
                        } else {
                            columns.push(id, game.into_owned());
                        }
                    }
                    let kept = columns.len();
                    let mut w = SerializedFileWriter::new(
                        Vec::new(),
                        GameColumns::schema(),
                        Arc::clone(properties),
                    )
                    .expect("writer");
                    if !columns.is_empty() {
                        columns.write_row_group(&mut w).expect("write row group");
                    }
                    let encoded = Bytes::from(w.into_inner().expect("encode"));
                    if tx.send((i, encoded, kept, bots, unrated_players)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut pending = BTreeMap::new();
        let mut written = 0;
        let (mut games, mut bots, mut unrated_players, mut empty_row_groups) =
            (0u64, 0u64, 0u64, 0usize);
        while written < num_row_groups {
            let (i, encoded, kept, row_group_bots, row_group_unrated_players) =
                rx.recv().expect("workers alive");
            bots += row_group_bots;
            unrated_players += row_group_unrated_players;
            pending.insert(i, (encoded, kept));
            while let Some((encoded, kept)) = pending.remove(&written) {
                let rows = input.num_rows(written) as usize;
                append_row_group(&mut writer, &encoded, &vec![true; kept]).expect("append");
                games += kept as u64;
                empty_row_groups += usize::from(kept == 0);
                progress.inc(rows as u64);
                written += 1;
                let _ = token_tx.send(());
            }
        }
        drop(token_tx);
        (games, bots, unrated_players, empty_row_groups)
    });

    writer.close().expect("close output");
    progress.finish_and_clear();
    let in_len = std::fs::metadata(&input_path).expect("input len").len();
    let out_len = std::fs::metadata(&out_path).expect("output len").len();
    println!(
        "{games} games, {bots} bots skipped, {unrated_players} without ratings skipped, {} of {num_row_groups} row groups, {:.2} GiB -> {:.2} GiB ({:.1}%), {:.1?}",
        num_row_groups - empty_row_groups,
        in_len as f64 / (1u64 << 30) as f64,
        out_len as f64 / (1u64 << 30) as f64,
        100.0 * out_len as f64 / in_len as f64,
        started.elapsed()
    );
}
