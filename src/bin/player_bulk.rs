use std::{collections::HashMap, fs, path::PathBuf, process, time::Instant};

use clap::{Args, Parser, Subcommand};
use lila_openingexplorer4::{
    mock_lila::{Fixture, Noise, prepare::name_hash, synth::Synth},
    parquet::Input,
    player::{
        PlayerDb, PlayerDbConfig, UserHasher,
        bulk::{
            USER_HASH_KEY_FILE,
            age::{AgeConfig, age},
            build::{AgingPlan, BuildConfig, CFS, build, sst_files},
            check::{CheckConfig, check},
            load_user_hasher,
            population::{
                Origin, SelectConfig, count_games, read_counts, read_population, select,
                write_counts, write_population,
            },
            scatter::{Partitions, ScatterConfig, read_names, scatter},
        },
    },
    util::{ByteSize, civil::days_from_civil},
};
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

/// Builds a prod-like personal explorer database for benchmarks, offline
/// from the parquet (milestone 4 of player-explorer.md): `count` and
/// `select` choose the population, `scatter` writes their games to
/// partitions, `build` writes the base as SST files and the tails, `ingest`
/// creates the database from the base, `age` runs the tails through the
/// live write path, `checkpoint` keeps the result.
///
/// Records and times are synthesized with the rules of the mock lila
/// (same --seed, --casual, noise and --data-end as `mock_lila serve`), so
/// that the database agrees with what the mock serves.
///
/// Logging is configured with BULK_LOG (like RUST_LOG), default info.
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Counts the games of all players of the parquet.
    Count(CountArgs),
    /// Selects the population.
    Select(SelectArgs),
    /// Writes the games of the population to partitions.
    Scatter(ScatterArgs),
    /// Writes the base SST files and the tails.
    Build(BuildArgs),
    /// Creates the database from the base SST files.
    Ingest(IngestArgs),
    /// Runs the tails through the live write path.
    Age(AgeArgs),
    /// Creates a checkpoint of a database.
    Checkpoint(CheckpointArgs),
    /// Lists the live SST files of a database (read-only): column family,
    /// level, file name, bytes.
    Files(FilesArgs),
    /// Checks a sample of the fixture players against the mock and a naive
    /// aggregation.
    Check(CheckArgs),
}

#[derive(Args)]
struct InputArgs {
    /// Parquet file(s) of pgn_to_parquet.
    #[arg(long, required = true)]
    parquet: Vec<PathBuf>,
    #[arg(long, default_value_t = 28)]
    threads: usize,
    /// Skip the first row groups (for slices).
    #[arg(long, default_value_t = 0)]
    skip_row_groups: usize,
    /// Use only this many row groups (for slices).
    #[arg(long)]
    row_groups: Option<usize>,
}

impl InputArgs {
    fn open(&self) -> Input {
        Input::open(&self.parquet, self.skip_row_groups, self.row_groups, false)
            .unwrap_or_else(|err| fail(err))
    }
}

#[derive(Args)]
struct CountArgs {
    #[command(flatten)]
    input: InputArgs,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Args)]
struct SelectArgs {
    #[arg(long)]
    counts: PathBuf,
    /// Fixture of the mock lila, whose players are all selected.
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long)]
    out: PathBuf,
    /// Players in the population (old production: 1.28 M).
    #[arg(long, default_value_t = 1280000)]
    players: usize,
    /// Mean games of the sampled players.
    #[arg(long, default_value_t = 2500.0)]
    mean_games: f64,
    #[arg(long, default_value_t = 1)]
    min_games: u32,
    #[arg(long, default_value_t = 0)]
    seed: u64,
}

/// The rules of the mock lila (same defaults as `mock_lila serve`).
#[derive(Args)]
struct SynthArgs {
    #[arg(long, default_value_t = 0, help_heading = "Mock")]
    seed: u64,
    #[arg(long, default_value_t = 0.5, help_heading = "Mock")]
    casual: f64,
    #[arg(long, default_value_t = 0.002, help_heading = "Mock")]
    aborted: f64,
    #[arg(long, default_value_t = 0.001, help_heading = "Mock")]
    no_start: f64,
    #[arg(long, default_value_t = 0.001, help_heading = "Mock")]
    no_account: f64,
    #[arg(long, default_value_t = 0.001, help_heading = "Mock")]
    no_rating: f64,
    #[arg(long, default_value_t = 0.001, help_heading = "Mock")]
    illegal_san: f64,
    /// As for `mock_lila serve`.
    #[arg(long, default_value_t = 1793825340000, help_heading = "Mock")]
    data_end: u64,
    /// Fixture of the mock, for the time shift.
    #[arg(long, help_heading = "Mock")]
    fixture: PathBuf,
}

impl SynthArgs {
    fn synth(&self) -> Synth {
        let fixture = Fixture::open(&self.fixture).unwrap_or_else(|err| fail(err));
        Synth {
            seed: self.seed,
            casual: self.casual,
            shift: Synth::shift_for(fixture.meta.max_created_at, self.data_end),
        }
    }

    fn noise(&self) -> Noise {
        Noise {
            aborted: self.aborted,
            no_start: self.no_start,
            no_account: self.no_account,
            no_rating: self.no_rating,
            illegal_san: self.illegal_san,
            not_found: 0.0,
        }
    }
}

#[derive(Args)]
struct ScatterArgs {
    #[command(flatten)]
    input: InputArgs,
    #[arg(long)]
    population: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 10)]
    partition_bits: u32,
    /// Also convert all standard games via the mock export and compare.
    #[arg(long)]
    verify: bool,
    #[command(flatten)]
    synth: SynthArgs,
}

#[derive(Args)]
struct BuildArgs {
    #[arg(long)]
    scatter: PathBuf,
    #[arg(long)]
    population: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 10)]
    partition_bits: u32,
    #[arg(long, default_value_t = 28)]
    threads: usize,
    /// Start of the aging window, as YYYY-MM-DD in database time (after the
    /// shift of the mock).
    #[arg(long)]
    window_start: String,
    /// End of the aging window (default: a day before --data-end, before
    /// requests.tsv starts).
    #[arg(long)]
    window_end: Option<String>,
    #[arg(long, default_value_t = 0.2)]
    requests_unindexed: f64,
    #[arg(long, default_value_t = 0.2)]
    requests_dormant: f64,
    #[arg(long, default_value_t = 0.1)]
    requests_new: f64,
    #[arg(long, default_value_t = 0.1)]
    sample_new: f64,
    #[arg(long, default_value_t = 0.3)]
    sample_active: f64,
    /// Mean days from the last index run of dormant players to the start of
    /// the window.
    #[arg(long, default_value_t = 180.0)]
    dormant_delay_days: f64,
    /// Bounds of the visit interval of active players, in days.
    #[arg(long, default_value_t = 1.0)]
    min_interval_days: f64,
    #[arg(long, default_value_t = 60.0)]
    max_interval_days: f64,
    #[arg(long, default_value = "256m")]
    sst_target: ByteSize,
    /// Only plan and count.
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    synth: SynthArgs,
}

#[derive(Args)]
struct DbArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "4g")]
    cache: ByteSize,
    #[arg(long, default_value_t = 4)]
    background_jobs: i32,
}

impl DbArgs {
    fn open(&self) -> PlayerDb {
        self.open_with(None)
    }

    /// Opens the database, created with the given user hasher if new.
    fn open_with(&self, user_hasher: Option<UserHasher>) -> PlayerDb {
        PlayerDb::open(
            &self.db,
            PlayerDbConfig {
                cache_bytes: self.cache.into(),
                background_jobs: self.background_jobs,
                user_hasher,
                ..PlayerDbConfig::default()
            },
        )
        .unwrap_or_else(|err| fail(err))
    }
}

#[derive(Args)]
struct IngestArgs {
    /// Output directory of `build`.
    #[arg(long)]
    build: PathBuf,
    #[command(flatten)]
    db: DbArgs,
}

#[derive(Args)]
struct AgeArgs {
    /// Output directory of `build`.
    #[arg(long)]
    build: PathBuf,
    /// Scatter directory (for the names).
    #[arg(long)]
    scatter: PathBuf,
    #[command(flatten)]
    db: DbArgs,
    #[arg(long, default_value_t = 10)]
    partition_bits: u32,
    #[arg(long, default_value_t = 16)]
    workers: usize,
    #[arg(long, default_value_t = 2000)]
    chunk_games: usize,
    #[arg(long, default_value_t = 1000)]
    rebuild_min_games: u32,
    #[arg(long, default_value_t = 0.5)]
    rebuild_factor: f64,
    /// Stop after the visits up to this date (YYYY-MM-DD, database time).
    #[arg(long)]
    until: Option<String>,
    /// Wait for compactions at the end.
    #[arg(long)]
    wait: bool,
    #[command(flatten)]
    synth: SynthArgs,
}

#[derive(Args)]
struct CheckArgs {
    #[command(flatten)]
    db: DbArgs,
    #[arg(long, default_value_t = 200)]
    players: usize,
    #[arg(long, default_value_t = 100000)]
    max_naive_games: u32,
    #[command(flatten)]
    synth: SynthArgs,
}

#[derive(Args)]
struct FilesArgs {
    #[arg(long)]
    db: PathBuf,
}

#[derive(Args)]
struct CheckpointArgs {
    #[command(flatten)]
    db: DbArgs,
    #[arg(long)]
    out: PathBuf,
}

fn fail(err: impl std::fmt::Display) -> ! {
    eprintln!("error: {err}");
    process::exit(1);
}

fn parse_date(s: &str) -> u64 {
    let parts: Vec<i64> = s.split('-').map(|p| p.parse().expect("date")).collect();
    let [y, m, d] = parts[..] else {
        fail(format!("expected YYYY-MM-DD, got {s}"));
    };
    days_from_civil(y, m as u32, d as u32) as u64 * 86_400_000
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .filter_or("BULK_LOG", "info")
            .write_style("BULK_LOG_STYLE"),
    )
    .format_module_path(false)
    .format_target(false)
    .init();
    let started = Instant::now();

    match Cli::parse().command {
        Command::Count(args) => {
            let input = args.input.open();
            let counts = count_games(&input, args.input.threads).unwrap_or_else(|err| fail(err));
            let games: u64 = counts.iter().map(|c| u64::from(c.games)).sum();
            log::info!("{} players, {games} (game, side) pairs", counts.len());
            write_counts(&args.out, &counts).unwrap_or_else(|err| fail(err));
        }
        Command::Select(args) => {
            let counts = read_counts(&args.counts).unwrap_or_else(|err| fail(err));
            let fixture = Fixture::open(&args.fixture).unwrap_or_else(|err| fail(err));
            let requests: Vec<u64> = fixture
                .players()
                .map(|(id, _)| name_hash(id.as_str().as_bytes()))
                .collect();
            let (members, alpha) = select(
                &counts,
                &requests,
                &SelectConfig {
                    players: args.players,
                    mean_games: args.mean_games,
                    min_games: args.min_games,
                    seed: args.seed,
                },
            );
            let summary = |origin: Origin| {
                let mut games: Vec<u32> = members
                    .iter()
                    .filter(|m| m.origin == origin)
                    .map(|m| m.games)
                    .collect();
                games.sort_unstable();
                let n = games.len().max(1);
                let q = |f: f64| {
                    games
                        .get(((n - 1) as f64 * f) as usize)
                        .copied()
                        .unwrap_or(0)
                };
                let total: u64 = games.iter().map(|&g| u64::from(g)).sum();
                format!(
                    "{origin:?}: {} players, {total} games, mean {:.0}, p10 {} p50 {} p90 {} p99 {} max {}",
                    games.len(),
                    total as f64 / n as f64,
                    q(0.1),
                    q(0.5),
                    q(0.9),
                    q(0.99),
                    q(1.0)
                )
            };
            log::info!("sampled with weight games^{alpha:.3}");
            log::info!("{}", summary(Origin::Requests));
            log::info!("{}", summary(Origin::Sample));
            write_population(&args.out, &members).unwrap_or_else(|err| fail(err));
        }
        Command::Scatter(args) => {
            let input = args.input.open();
            let members = read_population(&args.population).unwrap_or_else(|err| fail(err));
            scatter(&ScatterConfig {
                input: &input,
                members: &members,
                out: &args.out,
                partitions: Partitions {
                    bits: args.partition_bits,
                },
                threads: args.input.threads,
                synth: args.synth.synth(),
                noise: args.synth.noise(),
                verify: args.verify,
            })
            .unwrap_or_else(|err| fail(err));
        }
        Command::Build(args) => {
            let members = read_population(&args.population).unwrap_or_else(|err| fail(err));
            let synth = args.synth.synth();
            let window_end = args
                .window_end
                .as_deref()
                .map_or_else(|| args.synth.data_end - 86_400_000, parse_date);
            let day = 86_400_000.0;
            build(&BuildConfig {
                scatter_dir: &args.scatter,
                out: &args.out,
                partitions: Partitions {
                    bits: args.partition_bits,
                },
                members: &members,
                threads: args.threads,
                plan: AgingPlan {
                    window_start: synth.unshifted(parse_date(&args.window_start)),
                    window_end: synth.unshifted(window_end),
                    requests_unindexed: args.requests_unindexed,
                    requests_dormant: args.requests_dormant,
                    requests_new: args.requests_new,
                    sample_new: args.sample_new,
                    sample_active: args.sample_active,
                    dormant_delay: (args.dormant_delay_days * day) as u64,
                    min_interval: (args.min_interval_days * day) as u64,
                    max_interval: (args.max_interval_days * day) as u64,
                    seed: args.synth.seed,
                },
                synth,
                sst_target_bytes: args.sst_target.into(),
                dry_run: args.dry_run,
            })
            .unwrap_or_else(|err| fail(err));
        }
        Command::Ingest(args) => {
            if args.db.db.exists() {
                fail(format!("{} exists", args.db.db.display()));
            }
            let hasher = load_user_hasher(&args.build.join(USER_HASH_KEY_FILE))
                .unwrap_or_else(|err| fail(err));
            let db = args.db.open_with(Some(hasher));
            for cf in CFS {
                let files = sst_files(&args.build, cf).unwrap_or_else(|err| fail(err));
                let t = Instant::now();
                let n = files.len();
                if n == 0 {
                    continue;
                }
                db.ingest(cf, files).unwrap_or_else(|err| fail(err));
                log::info!("ingested {n} files into {cf} in {:.1?}", t.elapsed());
            }
            for cf in CFS {
                log::info!(
                    "{cf}: {}",
                    db.cf_property(cf, "rocksdb.levelstats")
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                );
            }
        }
        Command::Age(args) => {
            let db = args.db.open();
            let names: HashMap<_, _> = read_names(&args.scatter).unwrap_or_else(|err| fail(err));
            let synth = args.synth.synth();
            age(
                &db,
                &names,
                &AgeConfig {
                    out: &args.build,
                    partitions: Partitions {
                        bits: args.partition_bits,
                    },
                    workers: args.workers,
                    chunk_games: args.chunk_games,
                    rebuild_min_games: args.rebuild_min_games,
                    rebuild_factor: args.rebuild_factor,
                    until: args
                        .until
                        .as_deref()
                        .map(|d| synth.unshifted(parse_date(d))),
                    synth,
                },
            )
            .unwrap_or_else(|err| fail(err));
            db.flush().unwrap_or_else(|err| fail(err));
            if args.wait {
                let t = Instant::now();
                db.wait_for_compactions().unwrap_or_else(|err| fail(err));
                log::info!("compactions done in {:.0?}", t.elapsed());
            }
            for cf in CFS {
                log::info!(
                    "{cf}:\n{}",
                    db.cf_property(cf, "rocksdb.levelstats")
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                );
            }
            if let Some(stats) = db.property("rocksdb.stats").ok().flatten() {
                let path = args.db.db.join("age_stats.txt");
                let _ = fs::write(&path, stats);
                log::info!("rocksdb.stats in {}", path.display());
            }
        }
        Command::Check(args) => {
            let db = args.db.open();
            let fixture = Fixture::open(&args.synth.fixture).unwrap_or_else(|err| fail(err));
            let stats = check(
                &db,
                &CheckConfig {
                    fixture: &fixture,
                    synth: args.synth.synth(),
                    noise: args.synth.noise(),
                    players: args.players,
                    max_naive_games: args.max_naive_games,
                    seed: args.synth.seed,
                },
            )
            .unwrap_or_else(|err| fail(err));
            log::info!("{stats:?}");
            if stats.game_mismatches > 0 || stats.position_mismatches > 0 {
                process::exit(2);
            }
        }
        Command::Files(args) => {
            let db = rocksdb::DB::open_cf_for_read_only(
                &rocksdb::Options::default(),
                &args.db,
                PlayerDb::column_families(),
                false,
            )
            .unwrap_or_else(|err| fail(err));
            for file in db.live_files().unwrap_or_else(|err| fail(err)) {
                println!(
                    "{}\t{}\t{}\t{}",
                    file.column_family_name,
                    file.level,
                    file.name.trim_start_matches('/'),
                    file.size
                );
            }
        }
        Command::Checkpoint(args) => {
            let db = args.db.open();
            db.checkpoint(&args.out).unwrap_or_else(|err| fail(err));
        }
    }
    log::info!("done in {:.0?}", started.elapsed());
}
