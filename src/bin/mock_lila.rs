use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    process,
    time::{Duration, SystemTime},
};

use clap::{Args, Parser, Subcommand};
use lila_openingexplorer4::{
    mock_lila::{
        Faults, MockConfig, MockLila, Noise,
        prepare::{PrepareConfig, prepare},
    },
    parquet::Input,
    player::UserId,
};
use tikv_jemallocator::Jemalloc;

#[global_allocator]
static ALLOC: Jemalloc = Jemalloc;

/// A stand-in for lila's games export (`/api/games/user/{id}`), for tests
/// and benchmarks of the personal opening explorer.
///
/// `prepare` writes a fixture with the games of a set of players from the
/// parquet. `serve` serves it, with synthesized casual and ongoing games,
/// a time shift, a clock, a rate limit per stream, counting of concurrent
/// streams, and fault injection.
///
/// Endpoints besides the export: `GET /mock/stats` (JSON counters),
/// `GET /mock/clock` and `PUT /mock/clock?now=MS` (mock time in ms).
///
/// Logging is configured with MOCK_LOG (like RUST_LOG), default info.
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Writes a fixture from the parquet.
    Prepare(PrepareArgs),
    /// Serves a fixture.
    Serve(ServeArgs),
}

#[derive(Args)]
struct PrepareArgs {
    /// Parquet file(s) of pgn_to_parquet.
    #[arg(long, required = true)]
    parquet: Vec<PathBuf>,
    /// File with one user name per line.
    #[arg(long)]
    players: PathBuf,
    /// Fixture directory to write.
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 30)]
    threads: usize,
    /// Skip the first row groups (for slices).
    #[arg(long, default_value_t = 0)]
    skip_row_groups: usize,
    /// Use only this many row groups (for slices).
    #[arg(long)]
    row_groups: Option<usize>,
}

#[derive(Args)]
struct ServeArgs {
    /// Fixture directory written by `prepare`.
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long, default_value = "127.0.0.1:9666")]
    bind: SocketAddr,
    /// The newest month of the fixture is shifted to end at this time, in
    /// ms since the epoch. Default: end of requests.tsv (2026-11-04 20:49
    /// UTC).
    #[arg(long, default_value_t = 1793825340000)]
    data_end: u64,
    /// Mock time at startup, in ms since the epoch (default --data-end).
    /// Games created later are not served yet; games running at that time
    /// are served as ongoing.
    #[arg(long)]
    clock_start: Option<u64>,
    /// Start the clock at the current real time instead.
    #[arg(long, conflicts_with = "clock_start")]
    clock_real: bool,
    /// Mock milliseconds per real millisecond.
    #[arg(long, default_value_t = 1.0)]
    clock_rate: f64,
    /// Games per second per stream (0 for unlimited). lila: about 300.
    #[arg(long, default_value_t = 0.0)]
    games_per_second: f64,
    /// Log an error and count a violation whenever more streams are open
    /// at once (0 to disable). Set to the --indexers of the explorer.
    #[arg(long, default_value_t = 0)]
    max_streams: usize,
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Share of games served as casual.
    #[arg(long, default_value_t = 0.5)]
    casual: f64,
    /// Rate of aborted games.
    #[arg(long, default_value_t = 0.002, help_heading = "Noise")]
    aborted: f64,
    /// Rate of games with status noStart.
    #[arg(long, default_value_t = 0.001, help_heading = "Noise")]
    no_start: f64,
    /// Rate of opponents without account.
    #[arg(long, default_value_t = 0.001, help_heading = "Noise")]
    no_account: f64,
    /// Rate of opponents without rating.
    #[arg(long, default_value_t = 0.001, help_heading = "Noise")]
    no_rating: f64,
    /// Rate of games with an illegal move.
    #[arg(long, default_value_t = 0.001, help_heading = "Noise")]
    illegal_san: f64,
    /// Rate of players answering 404 (closed accounts).
    #[arg(long, default_value_t = 0.002, help_heading = "Noise")]
    not_found: f64,
    /// Rate of requests that wait --ttfb-ms before answering.
    #[arg(long, default_value_t = 0.0, help_heading = "Faults")]
    slow_start: f64,
    #[arg(long, default_value_t = 5000, help_heading = "Faults")]
    ttfb_ms: u64,
    /// Rate of streams that stall for --stall-ms at a random game.
    #[arg(long, default_value_t = 0.0, help_heading = "Faults")]
    stall: f64,
    #[arg(long, default_value_t = 90000, help_heading = "Faults")]
    stall_ms: u64,
    /// Rate of streams cut off at a random game.
    #[arg(long, default_value_t = 0.0, help_heading = "Faults")]
    disconnect: f64,
    /// Rate of requests answered with 503.
    #[arg(long, default_value_t = 0.0, help_heading = "Faults")]
    server_error: f64,
    /// Rate of malformed lines.
    #[arg(long, default_value_t = 0.0, help_heading = "Faults")]
    malformed: f64,
    /// Tab separated log of all requests.
    #[arg(long)]
    request_log: Option<PathBuf>,
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .filter_or("MOCK_LOG", "info")
            .write_style("MOCK_LOG_STYLE"),
    )
    .format_module_path(false)
    .format_target(false)
    .init();

    match Cli::parse().command {
        Command::Prepare(args) => {
            let players: Vec<UserId> = match fs::read_to_string(&args.players) {
                Ok(text) => text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .filter_map(|line| match UserId::from_name(line) {
                        Ok(id) => Some(id),
                        Err(_) => {
                            log::warn!("skipping invalid user name {line:?}");
                            None
                        }
                    })
                    .collect(),
                Err(err) => {
                    eprintln!("error: {}: {err}", args.players.display());
                    process::exit(1);
                }
            };
            let input =
                match Input::open(&args.parquet, args.skip_row_groups, args.row_groups, false) {
                    Ok(input) => input,
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(1);
                    }
                };
            if let Err(err) = prepare(PrepareConfig {
                input: &input,
                players,
                out: &args.out,
                threads: args.threads,
            }) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Command::Serve(args) => {
            let clock_start = if args.clock_real {
                Some(
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .expect("time after epoch")
                        .as_millis() as u64,
                )
            } else {
                args.clock_start
            };
            let config = MockConfig {
                bind: args.bind,
                fixture: args.fixture,
                data_end: args.data_end,
                clock_start,
                clock_rate: args.clock_rate,
                games_per_second: args.games_per_second,
                max_streams: args.max_streams,
                seed: args.seed,
                casual: args.casual,
                noise: Noise {
                    aborted: args.aborted,
                    no_start: args.no_start,
                    no_account: args.no_account,
                    no_rating: args.no_rating,
                    illegal_san: args.illegal_san,
                    not_found: args.not_found,
                },
                faults: Faults {
                    slow_start: args.slow_start,
                    ttfb: Duration::from_millis(args.ttfb_ms),
                    stall: args.stall,
                    stall_duration: Duration::from_millis(args.stall_ms),
                    disconnect: args.disconnect,
                    server_error: args.server_error,
                    malformed: args.malformed,
                },
                request_log: args.request_log,
            };
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(async {
                match MockLila::start(config).await {
                    Ok(mock) => {
                        log::info!("listening on {}", mock.addr);
                        let _ = tokio::signal::ctrl_c().await;
                        log::info!("final stats: {:?}", mock.stats());
                    }
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(1);
                    }
                }
            });
        }
    }
}
