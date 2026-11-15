use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use clap::Parser;
use lila_openingexplorer4::{
    server::db::{COLD_CF, GAME_IDS_CF, GAMES_CF, POSITIONS_CF},
    sst::{FILTER_BITS_PER_KEY, SstConfig, SstFile},
};
use rocksdb::{DB, DBCompressionType, Options, ReadOptions};
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

/// Rewrites the SST files of one column family of an ingested explorer
/// database with different table options, for example to evaluate layouts
/// or to adopt new options without rebuilding from parquet. The output files
/// are named in key order, so that `explorer ingest` can take them.
///
/// Defaults match the writers of parquet_to_games and parquet_to_positions
/// (`SstConfig` of the column family), so that only the given options
/// change. Cold positions of small databases are written with zstd level 3
/// instead (`--zstd-level 3`).
#[derive(Parser)]
struct Args {
    /// Database directory created by `explorer ingest`.
    #[arg(long)]
    db: PathBuf,
    /// Column family: games, positions, cold_positions or game_ids.
    #[arg(long)]
    cf: String,
    /// Directory for the new SST files.
    #[arg(long)]
    out: PathBuf,
    /// Files rewritten in parallel.
    #[arg(long, default_value_t = 24)]
    threads: usize,
    /// Only rewrite every Nth file, for quick size estimates. The result is
    /// not a complete column family.
    #[arg(long, default_value_t = 1)]
    sample: usize,
    /// Block size.
    #[arg(long)]
    block_size: Option<usize>,
    /// Keys between restart points of data blocks, which are delta-encoded
    /// in between.
    #[arg(long)]
    block_restart_interval: Option<i32>,
    /// Size of index and filter partitions.
    #[arg(long, default_value_t = 4096)]
    metadata_block_size: usize,
    #[arg(long, default_value_t = 16)]
    index_restart_interval: i32,
    /// Store the keys and the values of each data block in separate
    /// sections.
    #[arg(long)]
    separate_key_value_in_data_block: Option<bool>,
    /// Bloom-equivalent bits per key of the ribbon filter, 0 for none.
    #[arg(long)]
    filter_bits: Option<f64>,
    #[arg(long)]
    zstd_level: Option<i32>,
    /// Maximum size of the zstd dictionary trained for each file, 0 for
    /// none.
    #[arg(long, default_value_t = 0)]
    zstd_dict: i32,
    /// With lz4, the zstd level does not matter.
    #[arg(long, value_enum, default_value_t = Compression::Zstd)]
    compression: Compression,
    /// Keep a compressed block only if it is at most this many bytes per
    /// KiB of input (1024 keeps any block that compresses at all). Defaults
    /// to the bar of the writers.
    #[arg(long)]
    max_compressed_bytes_per_kb: Option<i32>,
    #[arg(long)]
    format_version: Option<i32>,
    /// More block based table options, in RocksDB's options string syntax,
    /// for options without a setter, like
    /// `uniform_cv_threshold=0.2`.
    #[arg(long)]
    table_options: Option<String>,
}

#[derive(Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
enum Compression {
    Zstd,
    Lz4,
    None,
}

fn main() {
    let args = Args::parse();
    let base = SstConfig::for_cf(&args.cf).expect("column family");
    let filter_bits = args.filter_bits.unwrap_or(if base.filter {
        FILTER_BITS_PER_KEY
    } else {
        0.0
    });
    let zstd_level = match args.compression {
        Compression::None => 0,
        // Nonzero, so that compression options (including the bar) are set.
        Compression::Lz4 => 1,
        Compression::Zstd => args.zstd_level.unwrap_or(base.zstd_level),
    };

    let cfs = [GAMES_CF, POSITIONS_CF, COLD_CF, GAME_IDS_CF];
    let mut db_opts = Options::default();
    db_opts.set_use_direct_reads(true);
    db_opts.set_max_open_files(-1);
    let descriptors = cfs.iter().map(|name| {
        let mut opts = Options::default();
        opts.set_block_based_table_factory(
            &SstConfig::for_cf(name)
                .expect("explorer cf")
                .read_table_options(),
        );
        rocksdb::ColumnFamilyDescriptor::new(*name, opts)
    });
    let db =
        DB::open_cf_descriptors_read_only(&db_opts, &args.db, descriptors, false).expect("open db");
    let cf = db.cf_handle(&args.cf).expect("column family");

    let mut files: Vec<_> = db
        .live_files()
        .expect("live files")
        .into_iter()
        .filter(|file| file.column_family_name == args.cf)
        .collect();
    files.sort_by(|a, b| a.start_key.cmp(&b.start_key));
    let files: Vec<_> = files.into_iter().step_by(args.sample.max(1)).collect();
    std::fs::create_dir_all(&args.out).expect("create out");

    let sst = SstConfig {
        block_size: args.block_size.unwrap_or(base.block_size),
        zstd_level,
        // Set below, with the given bits per key.
        filter: false,
        separate_key_value_in_data_block: args
            .separate_key_value_in_data_block
            .unwrap_or(base.separate_key_value_in_data_block),
    };
    let mut table_opts = sst.table_options();
    if let Some(interval) = args.block_restart_interval {
        table_opts.set_block_restart_interval(interval);
    }
    table_opts.set_metadata_block_size(args.metadata_block_size);
    table_opts.set_index_block_restart_interval(args.index_restart_interval);
    if filter_bits > 0.0 {
        table_opts.set_ribbon_filter(filter_bits);
        table_opts.set_partition_filters(true);
        table_opts.set_optimize_filters_for_memory(true);
    }
    if let Some(version) = args.format_version {
        table_opts.set_format_version(version);
    }
    let mut opts = sst.options_with_table(&table_opts);
    if let Some(table_options) = &args.table_options {
        opts = opts
            .get_options_from_string(format!("block_based_table_factory={{{table_options}}}"))
            .expect("table options");
    }
    if args.compression == Compression::Lz4 {
        opts.set_compression_type(DBCompressionType::Lz4);
    }
    if zstd_level != 0 && args.zstd_dict > 0 {
        opts.set_compression_options(-14, zstd_level, 0, args.zstd_dict);
        opts.set_zstd_max_train_bytes(args.zstd_dict * 100);
    }
    if let Some(bar) = args.max_compressed_bytes_per_kb
        && zstd_level != 0
    {
        opts.set_compression_options_max_compressed_bytes_per_kb(bar);
    }

    let started = Instant::now();
    let next = AtomicUsize::new(0);
    // Files, keys, output file bytes.
    let totals = Mutex::new([0u64; 3]);
    thread::scope(|s| {
        for _ in 0..args.threads.min(files.len()).max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(i) else {
                        break;
                    };
                    let mut read_opts = ReadOptions::default();
                    read_opts.fill_cache(false);
                    read_opts.set_readahead_size(4 << 20);
                    let start = file.start_key.clone().expect("start key");
                    let end = file.end_key.clone().expect("end key");
                    let mut iter = db.raw_iterator_cf_opt(&cf, read_opts);
                    iter.seek(&start);
                    let mut out = SstFile::create(&opts, args.out.join(format!("{i:05}.sst")))
                        .expect("create sst");
                    let mut keys = 0;
                    while let Some(key) = iter.key() {
                        if key > end.as_slice() {
                            break;
                        }
                        out.put(key, iter.value().expect("value")).expect("put");
                        keys += 1;
                        iter.next();
                    }
                    iter.status().expect("iterator");
                    let size = out.finish().expect("finish");
                    let mut totals = totals.lock().expect("totals");
                    for (total, n) in totals.iter_mut().zip([1, keys, size]) {
                        *total += n;
                    }
                }
            });
        }
    });
    let [count, keys, bytes] = totals.into_inner().expect("totals");
    println!(
        "{}: {count} files, {keys} keys, {bytes} bytes (input {} bytes) in {:.1?}",
        args.cf,
        files.iter().map(|file| file.size as u64).sum::<u64>(),
        started.elapsed()
    );
}
