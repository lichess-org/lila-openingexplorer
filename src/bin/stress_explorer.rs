use std::{
    collections::HashSet,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    mem,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use clap::{ArgGroup, Parser};
use lila_openingexplorer4::packed::GameId;
use serde_json::{Map, Value};
use tokio::task::JoinSet;

/// Replays requests from a log against running explorer servers, and
/// reports throughput, latency percentiles and invalid responses.
///
/// The log has tab separated nginx log entries
/// `$msec\t$status\t$request_time\t$upstream_response_time\t$request_uri`,
/// and may be zstd compressed (`.zst`). Requests are sent in the order in
/// which they started (`$msec - $request_time`), and `--speed` replays them
/// with their original timing. Requests rejected by nginx (429) and
/// requests that do not match a route of the given servers are skipped.
///
/// Exits with status 1 if any response was invalid, a server error (5xx) or
/// another unexpected status, or if any request failed without a response.
/// Client errors (4xx) are expected, since logs have bad requests.
/// Streamed responses (/player) are read to the end.
#[derive(Parser)]
#[command(group(ArgGroup::new("servers").required(true).multiple(true)))]
struct Args {
    /// Request log.
    log: PathBuf,
    /// Base URL of the lichess explorer, for /lichess.
    #[arg(long, group = "servers")]
    lichess_url: Option<String>,
    /// Base URL of the masters explorer, for /masters and
    /// /masters/pgn/{id}.
    #[arg(long, group = "servers")]
    masters_url: Option<String>,
    /// Base URL of the player explorer, for /player.
    #[arg(long, group = "servers")]
    player_url: Option<String>,
    /// Concurrent requests, across all targets.
    #[arg(long, default_value_t = 32)]
    concurrency: usize,
    /// Skip the first requests of the log.
    #[arg(long, default_value_t = 0)]
    skip: usize,
    /// Replay at most this many requests (after warm-up).
    #[arg(long)]
    limit: Option<usize>,
    /// Requests to send before measuring, for example to fill caches.
    #[arg(long, default_value_t = 0)]
    warm_up: usize,
    /// Skip repeated requests, to measure responses that are not in the
    /// response cache.
    #[arg(long)]
    unique: bool,
    /// Timeout of each request in seconds.
    #[arg(long, default_value_t = 60)]
    timeout: u64,
    /// Send the measured requests at this rate (requests per second, across
    /// all targets), instead of as fast as possible. Latencies then count
    /// from when a request was due, including time waiting for one of the
    /// concurrent slots.
    #[arg(long, conflicts_with = "speed")]
    rate: Option<f64>,
    /// Send the measured requests at their original times, sped up by this
    /// factor. Latencies count from when a request
    /// was due, as with `--rate`.
    #[arg(long)]
    speed: Option<f64>,
    /// Print throughput and latencies of the last interval (in seconds) to
    /// stderr, also during warm-up.
    #[arg(long)]
    interval: Option<f64>,
    /// Write the latency of each measured request to this file, as lines
    /// `index\tserver\tstatus\tmicroseconds` (index in the measured
    /// requests; status 0 if the request failed).
    #[arg(long)]
    latency_log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Lichess,
    Masters,
    Player,
}

impl Kind {
    /// The server for the path, if it is one of the routes.
    fn route(path: &str) -> Option<Kind> {
        match path {
            "/lichess" => Some(Kind::Lichess),
            "/masters" => Some(Kind::Masters),
            "/player" => Some(Kind::Player),
            _ => path
                .strip_prefix("/masters/pgn/")
                .filter(|id| id.parse::<GameId>().is_ok())
                .map(|_| Kind::Masters),
        }
    }
}

struct Target {
    kind: Kind,
    name: &'static str,
    url: String,
}

struct Request {
    target: usize,
    /// Path with query string.
    path: String,
    /// Start time in the log, in seconds.
    start: f64,
    /// When to send the request, relative to the start of the replay (or as
    /// soon as possible).
    due: Option<Duration>,
}

/// Parses a log entry: the start time and the path with query string, or
/// `None` for malformed entries and requests that nginx rejected.
fn parse_entry(line: &str) -> Option<(f64, &str)> {
    let mut fields = line.split('\t');
    let msec: f64 = fields.next()?.parse().ok()?;
    let status = fields.next()?;
    let request_time: f64 = fields.next()?.parse().ok()?;
    let path = fields.nth(1)?;
    (status != "429").then_some((msec - request_time, path))
}

/// The requests for the targets, and the number of skipped lines.
fn read_requests(args: &Args, targets: &[Target]) -> io::Result<(Vec<Request>, usize)> {
    let file = fs::File::open(&args.log)?;
    let reader: Box<dyn Read> = if args.log.extension().is_some_and(|ext| ext == "zst") {
        Box::new(zstd::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut seen = HashSet::new();
    let mut requests = Vec::new();
    let mut skipped = 0;
    for line in BufReader::new(reader).lines() {
        let line = line?;
        let Some((start, line)) = parse_entry(&line) else {
            skipped += 1;
            continue;
        };
        let line = line.to_owned();
        let path = line.split_once('?').map_or(line.as_str(), |(path, _)| path);
        let Some(target) = Kind::route(path)
            .and_then(|kind| targets.iter().position(|target| target.kind == kind))
        else {
            skipped += 1;
            continue;
        };
        if args.unique && !seen.insert(line.clone()) {
            continue;
        }
        requests.push(Request {
            target,
            path: line,
            start,
            due: None,
        });
    }
    // The log is ordered by completion.
    requests.sort_by(|a, b| a.start.total_cmp(&b.start));
    let requests = requests
        .into_iter()
        .skip(args.skip)
        .take(
            args.warm_up
                .saturating_add(args.limit.unwrap_or(usize::MAX)),
        )
        .collect();
    Ok((requests, skipped))
}

/// Most examples of failed requests to report.
const MAX_EXAMPLES: usize = 5;

/// A request in the latency log.
#[derive(Clone)]
struct LogEntry {
    /// Index in the measured requests.
    index: usize,
    /// HTTP status, or 0 if the request failed.
    status: u16,
    latency: Duration,
}

#[derive(Default, Clone)]
struct Outcome {
    latencies: Vec<Duration>,
    log: Vec<LogEntry>,
    ok: usize,
    client_errors: usize,
    server_errors: usize,
    other_statuses: usize,
    /// Requests that failed without a response.
    failed: usize,
    /// Successful responses that are not valid explorer responses.
    invalid: usize,
    bytes: usize,
    examples: Vec<String>,
}

impl Outcome {
    /// Whether anything went wrong that should not, whatever the requests.
    fn has_errors(&self) -> bool {
        self.invalid > 0 || self.server_errors > 0 || self.other_statuses > 0 || self.failed > 0
    }

    fn example(&mut self, example: String) {
        if self.examples.len() < MAX_EXAMPLES {
            self.examples.push(example);
        }
    }

    fn merge(&mut self, other: &Outcome) {
        self.latencies.extend_from_slice(&other.latencies);
        self.log.extend_from_slice(&other.log);
        self.ok += other.ok;
        self.client_errors += other.client_errors;
        self.server_errors += other.server_errors;
        self.other_statuses += other.other_statuses;
        self.failed += other.failed;
        self.invalid += other.invalid;
        self.bytes += other.bytes;
        for example in &other.examples {
            self.example(example.clone());
        }
    }
}

fn count(obj: &Map<String, Value>, key: &str) -> Result<u64, String> {
    obj.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing {key}"))
}

fn stats(obj: &Map<String, Value>) -> Result<u64, String> {
    Ok(count(obj, "white")? + count(obj, "draws")? + count(obj, "black")?)
}

/// Checks the structure of an explorer response.
fn validate(body: &[u8]) -> Result<(), String> {
    let value: Value = serde_json::from_slice(body).map_err(|err| err.to_string())?;
    let obj = value.as_object().ok_or("not an object")?;
    let total = stats(obj)?;
    let moves = obj
        .get("moves")
        .and_then(Value::as_array)
        .ok_or("missing moves")?;
    let mut sum = 0;
    for m in moves {
        let m = m.as_object().ok_or("move not an object")?;
        sum += stats(m)?;
        m.get("uci").and_then(Value::as_str).ok_or("missing uci")?;
        let san = m.get("san").and_then(Value::as_str).ok_or("missing san")?;
        if san == "--" {
            return Err("null san".to_owned());
        }
    }
    if sum > total {
        return Err(format!("moves sum {sum} > total {total}"));
    }
    if !obj.contains_key("opening") {
        return Err("missing opening".to_owned());
    }
    for key in ["topGames", "recentGames"] {
        let Some(games) = obj.get(key) else {
            continue;
        };
        for game in games.as_array().ok_or("games not an array")? {
            let game = game.as_object().ok_or("game not an object")?;
            game.get("id").and_then(Value::as_str).ok_or("missing id")?;
            game.get("uci")
                .and_then(Value::as_str)
                .ok_or("missing uci")?;
            for color in ["white", "black"] {
                game.get(color)
                    .and_then(|player| player.get("name"))
                    .and_then(Value::as_str)
                    .ok_or("missing player name")?;
            }
        }
    }
    Ok(())
}

async fn request(
    client: &reqwest::Client,
    url: &str,
    index: usize,
    started: Instant,
    outcome: &mut Outcome,
    recent: &Mutex<Vec<Duration>>,
) {
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(err) => {
            outcome.failed += 1;
            outcome.log.push(LogEntry {
                index,
                status: 0,
                latency: started.elapsed(),
            });
            outcome.example(format!("{url}: {err}"));
            return;
        }
    };
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .map(|value| value.as_bytes().to_owned())
        .unwrap_or_default();
    let is_json = content_type.starts_with(b"application/json");
    let is_ndjson = content_type.starts_with(b"application/x-ndjson");
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(err) => {
            outcome.failed += 1;
            outcome.log.push(LogEntry {
                index,
                status: 0,
                latency: started.elapsed(),
            });
            outcome.example(format!("{url}: {err}"));
            return;
        }
    };
    let latency = started.elapsed();
    outcome.latencies.push(latency);
    outcome.log.push(LogEntry {
        index,
        status: status.as_u16(),
        latency,
    });
    recent.lock().expect("recent latencies").push(latency);
    outcome.bytes += body.len();
    if status.is_success() {
        outcome.ok += 1;
        // Masters PGNs are not JSON.
        let result = if is_json {
            validate(&body)
        } else if is_ndjson {
            body.split(|&b| b == b'\n')
                .filter(|line| !line.trim_ascii().is_empty())
                .try_for_each(validate)
        } else {
            Ok(())
        };
        if let Err(err) = result {
            outcome.invalid += 1;
            outcome.example(format!("{url}: {err}"));
        }
        return;
    }
    if status.is_client_error() {
        outcome.client_errors += 1;
    } else if status.is_server_error() {
        outcome.server_errors += 1;
    } else {
        outcome.other_statuses += 1;
    }
    outcome.example(format!(
        "{url}: {status} {}",
        String::from_utf8_lossy(&body[..body.len().min(200)])
    ));
}

/// Sends the requests with the given concurrency, and at the given rate (or
/// as fast as possible). Returns the outcome for each target.
async fn replay(
    client: &reqwest::Client,
    targets: Arc<[Target]>,
    requests: Arc<[Request]>,
    concurrency: usize,
    recent: Arc<Mutex<Vec<Duration>>>,
) -> Vec<Outcome> {
    let next = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();
    let mut tasks = JoinSet::new();
    for _ in 0..concurrency.max(1) {
        let client = client.clone();
        let targets = Arc::clone(&targets);
        let requests = Arc::clone(&requests);
        let next = Arc::clone(&next);
        let recent = Arc::clone(&recent);
        tasks.spawn(async move {
            let mut outcomes = vec![Outcome::default(); targets.len()];
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(req) = requests.get(i) else {
                    break;
                };
                let started = match req.due {
                    Some(due) => {
                        let due = start + due;
                        tokio::time::sleep_until(due.into()).await;
                        due
                    }
                    None => Instant::now(),
                };
                let url = format!("{}{}", targets[req.target].url, req.path);
                request(
                    &client,
                    &url,
                    i,
                    started,
                    &mut outcomes[req.target],
                    &recent,
                )
                .await;
            }
            outcomes
        });
    }
    let mut outcomes = vec![Outcome::default(); targets.len()];
    while let Some(task) = tasks.join_next().await {
        for (outcome, other) in outcomes.iter_mut().zip(task.expect("replay task")) {
            outcome.merge(&other);
        }
    }
    outcomes
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let i = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[i]
}

/// Prints the throughput and latencies of each interval, until aborted.
async fn report_intervals(interval: Duration, recent: Arc<Mutex<Vec<Duration>>>) {
    let start = Instant::now();
    let mut ticker = tokio::time::interval_at((start + interval).into(), interval);
    loop {
        ticker.tick().await;
        let mut latencies = mem::take(&mut *recent.lock().expect("recent latencies"));
        latencies.sort_unstable();
        eprintln!(
            "[{:.0}s] {:.0} req/s, p50 {:.2?}, p99 {:.2?}, max {:.2?}",
            start.elapsed().as_secs_f64(),
            latencies.len() as f64 / interval.as_secs_f64(),
            percentile(&latencies, 0.5),
            percentile(&latencies, 0.99),
            latencies.last().copied().unwrap_or_default(),
        );
    }
}

fn report(name: &str, outcome: &mut Outcome, elapsed: Duration) {
    outcome.latencies.sort_unstable();
    let latencies = &outcome.latencies;
    let n = latencies.len() + outcome.failed;
    println!(
        "{name}: {n} requests in {elapsed:.1?}: {:.0} req/s, {:.1} MiB",
        n as f64 / elapsed.as_secs_f64(),
        outcome.bytes as f64 / (1u64 << 20) as f64,
    );
    println!(
        "2xx {} (invalid {}), 4xx {}, 5xx {}, other {}, failed {}",
        outcome.ok,
        outcome.invalid,
        outcome.client_errors,
        outcome.server_errors,
        outcome.other_statuses,
        outcome.failed,
    );
    if !latencies.is_empty() {
        let mean = latencies.iter().sum::<Duration>() / latencies.len() as u32;
        println!(
            "latency mean {mean:.2?}, p50 {:.2?}, p90 {:.2?}, p99 {:.2?}, p99.9 {:.2?}, max {:.2?}",
            percentile(latencies, 0.5),
            percentile(latencies, 0.9),
            percentile(latencies, 0.99),
            percentile(latencies, 0.999),
            latencies.last().copied().unwrap_or_default(),
        );
    }
    for example in &outcome.examples {
        println!("example: {example}");
    }
}

fn write_latency_log(path: &PathBuf, targets: &[Target], outcomes: &[Outcome]) -> io::Result<()> {
    let mut entries: Vec<(&str, LogEntry)> = targets
        .iter()
        .zip(outcomes)
        .flat_map(|(target, outcome)| outcome.log.iter().map(|entry| (target.name, entry.clone())))
        .collect();
    entries.sort_unstable_by_key(|(_, entry)| entry.index);
    let mut out = io::BufWriter::new(fs::File::create(path)?);
    for (name, entry) in entries {
        writeln!(
            out,
            "{}\t{name}\t{}\t{}",
            entry.index,
            entry.status,
            entry.latency.as_micros()
        )?;
    }
    out.flush()
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let targets: Vec<Target> = [
        (Kind::Lichess, "lichess", &args.lichess_url),
        (Kind::Masters, "masters", &args.masters_url),
        (Kind::Player, "player", &args.player_url),
    ]
    .into_iter()
    .filter_map(|(kind, name, url)| {
        url.as_ref().map(|url| Target {
            kind,
            name,
            url: url.trim_end_matches('/').to_owned(),
        })
    })
    .collect();
    let (requests, skipped) = match read_requests(&args, &targets) {
        Ok(requests) => requests,
        Err(err) => {
            eprintln!("error: {}: {err}", args.log.display());
            std::process::exit(1);
        }
    };
    let warm_up = args.warm_up.min(requests.len());
    eprintln!(
        "{} requests from {} ({warm_up} warm-up, {skipped} lines skipped), concurrency {}",
        requests.len(),
        args.log.display(),
        args.concurrency,
    );
    for (i, target) in targets.iter().enumerate() {
        eprintln!(
            "  {} at {}: {} requests",
            target.name,
            target.url,
            requests.iter().filter(|req| req.target == i).count()
        );
    }
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(args.concurrency)
        .timeout(Duration::from_secs(args.timeout))
        .build()
        .expect("http client");
    let targets: Arc<[Target]> = targets.into();
    let mut requests = requests;
    let mut measured = requests.split_off(warm_up);
    if let Some(rate) = args.rate {
        for (i, req) in measured.iter_mut().enumerate() {
            req.due = Some(Duration::from_secs_f64(i as f64 / rate));
        }
    } else if let Some(speed) = args.speed {
        let first = measured.first().map_or(0.0, |req| req.start);
        for req in &mut measured {
            req.due = Some(Duration::from_secs_f64(
                (req.start - first).max(0.0) / speed,
            ));
        }
    }
    let measured: Arc<[Request]> = measured.into();
    let recent = Arc::new(Mutex::new(Vec::new()));
    let reporter = args.interval.map(|interval| {
        tokio::spawn(report_intervals(
            Duration::from_secs_f64(interval),
            Arc::clone(&recent),
        ))
    });

    if warm_up > 0 {
        let started = Instant::now();
        let outcomes = replay(
            &client,
            Arc::clone(&targets),
            requests.into(),
            args.concurrency,
            Arc::clone(&recent),
        )
        .await;
        eprintln!(
            "warmed up with {warm_up} requests in {:.1?} ({} failed)",
            started.elapsed(),
            outcomes.iter().map(|outcome| outcome.failed).sum::<usize>()
        );
    }

    let started = Instant::now();
    let mut outcomes = replay(
        &client,
        Arc::clone(&targets),
        measured,
        args.concurrency,
        recent,
    )
    .await;
    let elapsed = started.elapsed();
    if let Some(reporter) = reporter {
        reporter.abort();
    }
    if let Some(path) = &args.latency_log
        && let Err(err) = write_latency_log(path, &targets, &outcomes)
    {
        eprintln!("error: {}: {err}", path.display());
    }
    for (target, outcome) in targets.iter().zip(&mut outcomes) {
        report(target.name, outcome, elapsed);
    }
    if targets.len() > 1 {
        let mut total = Outcome::default();
        for outcome in &outcomes {
            total.merge(outcome);
        }
        report("total", &mut total, elapsed);
    }
    if outcomes.iter().any(Outcome::has_errors) {
        eprintln!("error: invalid responses, server errors or failed requests");
        std::process::exit(1);
    }
}
