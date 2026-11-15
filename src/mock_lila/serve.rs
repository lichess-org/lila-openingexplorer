//! Serves lila's games export (`/api/games/user/{id}`) from a fixture.

use std::{
    convert::Infallible,
    fs::File,
    io::{self, BufWriter, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use shakmaty::Color;
use tokio::{net::TcpListener, sync::mpsc, task};

use crate::{
    lila_variant::LilaVariant,
    mock_lila::{
        fixture::{Fixture, FixtureGame, FixturePlayer, record_bodies},
        prepare::mix,
        synth::{Synth, game_number, unit, user_fingerprint},
    },
    player::UserId,
};

/// Rates of synthesized irregularities, each in `0.0..=1.0`.
#[derive(Debug, Clone, Default)]
pub struct Noise {
    /// Games that are aborted.
    pub aborted: f64,
    /// Games with status `noStart`.
    pub no_start: f64,
    /// Games where the opponent has no account.
    pub no_account: f64,
    /// Games where the opponent has no rating.
    pub no_rating: f64,
    /// Games with an illegal move at a random ply.
    pub illegal_san: f64,
    /// Players that answer 404 (closed accounts).
    pub not_found: f64,
}

/// Rates of injected faults, each in `0.0..=1.0`.
#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// Requests that wait `ttfb` before answering.
    pub slow_start: f64,
    pub ttfb: Duration,
    /// Streams that stall for `stall` at a random game.
    pub stall: f64,
    pub stall_duration: Duration,
    /// Streams that are cut off at a random game.
    pub disconnect: f64,
    /// Requests answered with 503.
    pub server_error: f64,
    /// Lines that are not valid JSON.
    pub malformed: f64,
}

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub bind: SocketAddr,
    pub fixture: PathBuf,
    /// The newest month of the fixture is shifted to end at this time (ms).
    pub data_end: u64,
    /// Mock time at startup (ms), default `data_end`.
    pub clock_start: Option<u64>,
    /// Mock milliseconds per real millisecond.
    pub clock_rate: f64,
    /// Games per second per stream (0 for unlimited).
    pub games_per_second: f64,
    /// Log an error and count a violation when more streams are open at
    /// once (0 to disable).
    pub max_streams: usize,
    pub seed: u64,
    /// Share of games that are casual.
    pub casual: f64,
    pub noise: Noise,
    pub faults: Faults,
    /// Tab separated log of all requests.
    pub request_log: Option<PathBuf>,
}

impl Default for MockConfig {
    fn default() -> MockConfig {
        MockConfig {
            bind: "127.0.0.1:0".parse().expect("address"),
            fixture: PathBuf::new(),
            data_end: 0,
            clock_start: None,
            clock_rate: 1.0,
            games_per_second: 0.0,
            max_streams: 0,
            seed: 0,
            casual: 0.5,
            noise: Noise::default(),
            faults: Faults::default(),
            request_log: None,
        }
    }
}

/// Statistics of the mock, as served by `/mock/stats`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MockStats {
    pub requests: u64,
    pub not_found: u64,
    pub server_errors: u64,
    pub active_streams: u64,
    pub max_active_streams: u64,
    /// Times that more than `max_streams` streams were open.
    pub violations: u64,
    pub games_served: u64,
    pub completed_streams: u64,
    pub aborted_streams: u64,
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    not_found: AtomicU64,
    server_errors: AtomicU64,
    active: AtomicU64,
    max_active: AtomicU64,
    violations: AtomicU64,
    games: AtomicU64,
    completed: AtomicU64,
    aborted: AtomicU64,
}

struct Clock {
    base: Mutex<(Instant, u64)>,
    rate: f64,
}

impl Clock {
    fn now(&self) -> u64 {
        let (real, mock) = *self.base.lock().expect("clock");
        mock + (real.elapsed().as_secs_f64() * 1000.0 * self.rate) as u64
    }

    fn set(&self, now: u64) {
        *self.base.lock().expect("clock") = (Instant::now(), now);
    }
}

struct MockState {
    config: MockConfig,
    fixture: Fixture,
    synth: Synth,
    clock: Clock,
    counters: Counters,
    log: Option<Mutex<BufWriter<File>>>,
    request_number: AtomicU64,
}

/// One open stream (or request), for counting concurrent streams.
struct StreamGuard(Arc<MockState>);

impl StreamGuard {
    fn new(state: &Arc<MockState>) -> StreamGuard {
        let active = state.counters.active.fetch_add(1, Ordering::SeqCst) + 1;
        state
            .counters
            .max_active
            .fetch_max(active, Ordering::SeqCst);
        let max = state.config.max_streams as u64;
        if max > 0 && active > max {
            state.counters.violations.fetch_add(1, Ordering::SeqCst);
            log::error!("VIOLATION: {active} concurrent streams, more than {max}");
        }
        StreamGuard(Arc::clone(state))
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.counters.active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
struct LilaUser<'a> {
    name: &'a str,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
}

#[derive(Serialize)]
struct LilaPlayer<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<LilaUser<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rating: Option<u16>,
}

#[derive(Serialize)]
struct LilaPlayers<'a> {
    white: LilaPlayer<'a>,
    black: LilaPlayer<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LilaGame<'a> {
    id: String,
    rated: bool,
    variant: &'a str,
    speed: &'a str,
    perf: &'a str,
    created_at: u64,
    last_move_at: u64,
    status: &'a str,
    source: &'a str,
    players: LilaPlayers<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    winner: Option<&'a str>,
    moves: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_fen: Option<&'a str>,
}

/// What became of a game at the current time.
pub enum Presented {
    /// Not created yet.
    Future,
    /// Ongoing, but ongoing games were not requested.
    Hidden,
    Line(Vec<u8>),
}

impl MockState {
    fn shifted(&self, created_at: u64) -> u64 {
        self.synth.shifted(created_at)
    }

    fn unshifted(&self, created_at: u64) -> u64 {
        self.synth.unshifted(created_at)
    }

    fn present(&self, game: &FixtureGame, user: &UserId, now: u64, ongoing: bool) -> Presented {
        present(&self.synth, &self.config.noise, game, user, now, ongoing)
    }

    fn log_request(&self, line: std::fmt::Arguments<'_>) {
        if let Some(log) = &self.log {
            let mut log = log.lock().expect("request log");
            let _ = log.write_fmt(line);
            let _ = log.write_all(b"\n");
            let _ = log.flush();
        }
    }
}

#[derive(Deserialize)]
struct ExportQuery {
    since: Option<u64>,
    until: Option<u64>,
    max: Option<u64>,
    sort: Option<String>,
    ongoing: Option<bool>,
}

/// The export line of a game for the user at mock time `now` (with shifted
/// times), as the mock serves it.
pub fn present(
    synth: &Synth,
    noise: &Noise,
    game: &FixtureGame,
    user: &UserId,
    now: u64,
    ongoing: bool,
) -> Presented {
    let seed = synth.seed;
    let id = game_number(game.id);
    let created_at = synth.shifted(game.created_at);
    if created_at > now {
        return Presented::Future;
    }
    let plies = game.plies() as u64;
    let duration = synth.duration(id, plies, game.speed);
    let finished_at = created_at + duration;
    let is_ongoing = now < finished_at;
    if is_ongoing && !ongoing {
        return Presented::Hidden;
    }

    let mut sans: Vec<&str> = game.sans.split_ascii_whitespace().collect();
    let mut status = game.status.key();
    let mut winner = game.outcome.winner();
    let mut last_move_at = finished_at;
    if is_ongoing {
        let done = (now - created_at) as f64 / duration as f64;
        sans.truncate((plies as f64 * done) as usize);
        status = "started";
        winner = None;
        last_move_at = now;
    } else if unit(seed, id, 2) < noise.aborted {
        sans.truncate(1);
        status = "aborted";
        winner = None;
    } else if unit(seed, id, 3) < noise.no_start {
        sans.clear();
        status = "noStart";
        winner = None;
    }
    if unit(seed, id, 4) < noise.illegal_san && sans.len() > 2 {
        let ply = 1 + (unit(seed, id, 5) * (sans.len() - 1) as f64) as usize;
        sans[ply] = "O-O-O";
    }
    let moves = sans.join(" ");

    let me = Color::ALL
        .into_iter()
        .find(|&c| user.is_name(&game.players.get(c).name))
        .unwrap_or(Color::White);
    let player = |color: Color| -> LilaPlayer<'_> {
        let p: &FixturePlayer = game.players.get(color);
        let opponent = color != me;
        LilaPlayer {
            user: (!(opponent && unit(seed, id, 6) < noise.no_account)).then(|| LilaUser {
                name: &p.name,
                id: p.name.to_ascii_lowercase(),
                title: p.title.as_deref(),
            }),
            rating: p
                .rating
                .filter(|_| !(opponent && unit(seed, id, 7) < noise.no_rating)),
        }
    };
    let rated = synth.rated(id);
    let variant = game.variant.key();
    let speed = game.speed.as_str();
    let perf = match game.variant {
        LilaVariant::Standard | LilaVariant::FromPosition => speed,
        other => other.key(),
    };
    let json = LilaGame {
        id: game.id.to_string(),
        rated,
        variant,
        speed,
        perf,
        created_at,
        last_move_at,
        status,
        source: "pool",
        players: LilaPlayers {
            white: player(Color::White),
            black: player(Color::Black),
        },
        winner: winner.map(|c| match c {
            Color::White => "white",
            Color::Black => "black",
        }),
        moves: &moves,
        initial_fen: game.initial_fen.as_deref(),
    };
    let mut line = serde_json::to_vec(&json).expect("serialize game");
    line.push(b'\n');
    Presented::Line(line)
}

/// Answers `/api/games/user/{id}`.
async fn export(
    State(state): State<Arc<MockState>>,
    Path(user): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let guard = StreamGuard::new(&state);
    let number = state.request_number.fetch_add(1, Ordering::Relaxed);
    state.counters.requests.fetch_add(1, Ordering::Relaxed);
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let faults = state.config.faults.clone();
    let seed = state.config.seed ^ mix(number);
    let since = query.since.unwrap_or(0);
    let until = query.until.unwrap_or(u64::MAX);

    if unit(seed, 0, 10) < faults.slow_start {
        tokio::time::sleep(faults.ttfb).await;
    }
    if unit(seed, 0, 11) < faults.server_error {
        state.counters.server_errors.fetch_add(1, Ordering::Relaxed);
        state.log_request(format_args!(
            "{started}\t{user}\t{since}\t{until}\t503\t0\t0\tfault"
        ));
        return (StatusCode::SERVICE_UNAVAILABLE, "injected fault").into_response();
    }
    let games = UserId::from_name(&user).ok().and_then(|id| {
        let closed =
            unit(state.config.seed, user_fingerprint(&id), 12) < state.config.noise.not_found;
        state
            .fixture
            .player(&id)
            .filter(|_| !closed)
            .map(|games| (id, games))
    });
    let Some((user, games)) = games else {
        state.counters.not_found.fetch_add(1, Ordering::Relaxed);
        state.log_request(format_args!(
            "{started}\t{user}\t{since}\t{until}\t404\t0\t0\tnot found"
        ));
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    let desc = query.sort.as_deref() == Some("dateDesc");
    let ongoing = query.ongoing.unwrap_or(false);
    let max = query.max.unwrap_or(u64::MAX);
    let stall_at = (unit(seed, 0, 13) < faults.stall).then(|| unit(seed, 0, 14));
    let disconnect_at = (unit(seed, 0, 15) < faults.disconnect).then(|| unit(seed, 0, 16));
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    let task_state = Arc::clone(&state);
    task::spawn(async move {
        let state = task_state;
        let _guard = guard;
        let real_start = Instant::now();
        let gps = state.config.games_per_second;
        let lo = state.fixture.lower_bound(games, state.unshifted(since));
        let hi = state
            .fixture
            .lower_bound(games, state.unshifted(until).saturating_add(1));
        let total = hi.saturating_sub(lo);
        let stall_game = stall_at.map(|f| lo + (f * total as f64) as u64);
        let disconnect_game = disconnect_at.map(|f| lo + (f * total as f64) as u64);
        let mut served = 0u64;
        let mut reason = "complete";
        let mut pos = if desc { hi } else { lo };
        let now = state.clock.now();
        'outer: while served < max && if desc { pos > lo } else { pos < hi } {
            let (start, end) = if desc {
                (pos.saturating_sub(256).max(lo), pos)
            } else {
                (pos, (pos + 256).min(hi))
            };
            pos = if desc { start } else { end };
            let fixture_state = Arc::clone(&state);
            let records =
                match task::spawn_blocking(move || fixture_state.fixture.read_records(start, end))
                    .await
                    .expect("read records")
                {
                    Ok(records) => records,
                    Err(err) => {
                        log::error!("reading fixture: {err}");
                        reason = "io error";
                        break;
                    }
                };
            let mut bodies: Vec<&[u8]> = record_bodies(&records).collect();
            if desc {
                bodies.reverse();
            }
            for (i, body) in bodies.into_iter().enumerate() {
                let number = if desc {
                    end - 1 - i as u64
                } else {
                    start + i as u64
                };
                if disconnect_game == Some(number) {
                    let _ = tx.send(Err(io::Error::other("injected disconnect"))).await;
                    reason = "fault disconnect";
                    break 'outer;
                }
                if stall_game == Some(number) {
                    tokio::select! {
                        _ = tokio::time::sleep(faults.stall_duration) => (),
                        _ = tx.closed() => {
                            reason = "client gone";
                            break 'outer;
                        }
                    }
                }
                let game = match FixtureGame::unpack_body(body) {
                    Ok(game) => game,
                    Err(err) => {
                        log::error!("corrupt fixture record: {err}");
                        continue;
                    }
                };
                let line = match state.present(&game, &user, now, ongoing) {
                    Presented::Future if desc => continue,
                    Presented::Future => break 'outer,
                    Presented::Hidden => continue,
                    Presented::Line(line) => line,
                };
                let line = if unit(seed, number, 17) < faults.malformed {
                    b"{\"id\":\"broken\n".to_vec()
                } else {
                    line
                };
                if gps > 0.0 {
                    let due = real_start + Duration::from_secs_f64(served as f64 / gps);
                    if due > Instant::now() {
                        tokio::select! {
                            _ = tokio::time::sleep_until(due.into()) => (),
                            _ = tx.closed() => {
                                reason = "client gone";
                                break 'outer;
                            }
                        }
                    }
                }
                if tx.send(Ok(Bytes::from(line))).await.is_err() {
                    reason = "client gone";
                    break 'outer;
                }
                served += 1;
                state.counters.games.fetch_add(1, Ordering::Relaxed);
                if served >= max {
                    break 'outer;
                }
            }
        }
        if reason == "complete" {
            state.counters.completed.fetch_add(1, Ordering::Relaxed);
        } else {
            state.counters.aborted.fetch_add(1, Ordering::Relaxed);
        }
        let elapsed = real_start.elapsed();
        log::debug!(
            "{user} since {since}: {served} games in {:.3?} ({reason})",
            elapsed
        );
        state.log_request(format_args!(
            "{started}\t{user}\t{since}\t{until}\t200\t{served}\t{}\t{reason}",
            elapsed.as_millis()
        ));
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from_stream({
            let mut rx = rx;
            futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
        }))
        .expect("response")
}

async fn stats(State(state): State<Arc<MockState>>) -> Json<MockStats> {
    let c = &state.counters;
    Json(MockStats {
        requests: c.requests.load(Ordering::SeqCst),
        not_found: c.not_found.load(Ordering::SeqCst),
        server_errors: c.server_errors.load(Ordering::SeqCst),
        active_streams: c.active.load(Ordering::SeqCst),
        max_active_streams: c.max_active.load(Ordering::SeqCst),
        violations: c.violations.load(Ordering::SeqCst),
        games_served: c.games.load(Ordering::SeqCst),
        completed_streams: c.completed.load(Ordering::SeqCst),
        aborted_streams: c.aborted.load(Ordering::SeqCst),
    })
}

#[derive(Deserialize)]
struct ClockQuery {
    now: u64,
}

async fn get_clock(State(state): State<Arc<MockState>>) -> String {
    state.clock.now().to_string()
}

async fn set_clock(
    State(state): State<Arc<MockState>>,
    Query(query): Query<ClockQuery>,
) -> Result<String, Infallible> {
    state.clock.set(query.now);
    Ok(query.now.to_string())
}

/// A running mock lila.
pub struct MockLila {
    pub addr: SocketAddr,
    state: Arc<MockState>,
}

impl MockLila {
    /// Opens the fixture and starts serving in the background.
    pub async fn start(config: MockConfig) -> io::Result<MockLila> {
        let fixture = Fixture::open(&config.fixture).map_err(io::Error::other)?;
        let synth = Synth {
            seed: config.seed,
            casual: config.casual,
            shift: Synth::shift_for(fixture.meta.max_created_at, config.data_end),
        };
        let log = config
            .request_log
            .as_ref()
            .map(|path| {
                let mut file = BufWriter::new(File::create(path)?);
                writeln!(
                    file,
                    "started\tplayer\tsince\tuntil\tstatus\tgames\tms\treason"
                )?;
                Ok::<_, io::Error>(Mutex::new(file))
            })
            .transpose()?;
        let state = Arc::new(MockState {
            clock: Clock {
                base: Mutex::new((
                    Instant::now(),
                    config.clock_start.unwrap_or(config.data_end),
                )),
                rate: config.clock_rate,
            },
            config,
            fixture,
            synth,
            counters: Counters::default(),
            log,
            request_number: AtomicU64::new(0),
        });
        log::info!(
            "mock lila: {} players, {} games, shift {:.1} days, clock at {}",
            state.fixture.meta.players,
            state.fixture.meta.games,
            state.synth.shift as f64 / 86_400_000.0,
            state.clock.now()
        );
        let app = Router::new()
            .route("/api/games/user/{user}", get(export))
            .route("/mock/stats", get(stats))
            .route("/mock/clock", get(get_clock).put(set_clock))
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind(state.config.bind).await?;
        let addr = listener.local_addr()?;
        task::spawn(async move {
            if let Err(err) = axum::serve(listener, app).await {
                log::error!("mock lila: {err}");
            }
        });
        Ok(MockLila { addr, state })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn stats(&self) -> MockStats {
        let c = &self.state.counters;
        MockStats {
            requests: c.requests.load(Ordering::SeqCst),
            not_found: c.not_found.load(Ordering::SeqCst),
            server_errors: c.server_errors.load(Ordering::SeqCst),
            active_streams: c.active.load(Ordering::SeqCst),
            max_active_streams: c.max_active.load(Ordering::SeqCst),
            violations: c.violations.load(Ordering::SeqCst),
            games_served: c.games.load(Ordering::SeqCst),
            completed_streams: c.completed.load(Ordering::SeqCst),
            aborted_streams: c.aborted.load(Ordering::SeqCst),
        }
    }

    /// The mock's current time (ms).
    pub fn now(&self) -> u64 {
        self.state.clock.now()
    }

    pub fn set_now(&self, now: u64) {
        self.state.clock.set(now);
    }

    /// `createdAt` as served for a `createdAt` of the fixture.
    pub fn shifted(&self, created_at: u64) -> u64 {
        self.state.shifted(created_at)
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::{ByColor, KnownOutcome};

    use super::*;
    use crate::{
        mock_lila::{
            FixtureWriter,
            fixture::{FinalStatus, FixtureGame},
            synth::next_month_start,
        },
        packed::Speed,
        util::civil::{civil_from_days, days_from_civil},
    };

    const DAY: u64 = 86_400_000;

    /// A fixture with games of alice every day of 2026-09, at noon.
    fn fixture(dir: &std::path::Path) -> u64 {
        let mut writer = FixtureWriter::create(dir).unwrap();
        let start = days_from_civil(2026, 9, 1) as u64 * DAY + DAY / 2;
        let records: Vec<Vec<u8>> = (0..30u64)
            .map(|day| {
                let game = FixtureGame {
                    created_at: start + day * DAY,
                    id: format!("game{day:04}").parse().unwrap(),
                    variant: LilaVariant::Standard,
                    speed: Speed::Blitz,
                    outcome: KnownOutcome::Draw,
                    status: FinalStatus::Draw,
                    players: ByColor {
                        white: FixturePlayer {
                            name: "Alice".to_owned(),
                            title: None,
                            rating: Some(1500),
                        },
                        black: FixturePlayer {
                            name: "Bob".to_owned(),
                            title: None,
                            rating: Some(1600),
                        },
                    },
                    initial_fen: None,
                    sans: "e4 e5 Nf3 Nc6".to_owned(),
                };
                let mut buf = Vec::new();
                game.pack(&mut buf);
                buf
            })
            .collect();
        writer
            .write_player(
                &UserId::from_name("alice").unwrap(),
                records.iter().map(Vec::as_slice),
            )
            .unwrap();
        writer.finish().unwrap();
        start
    }

    async fn lines(url: &str) -> (StatusCode, Vec<serde_json::Value>) {
        let response = reqwest::get(url).await.unwrap();
        let status = StatusCode::from_u16(response.status().as_u16()).unwrap();
        let text = response.text().await.unwrap();
        (
            status,
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect(),
        )
    }

    #[tokio::test]
    async fn test_export() {
        let dir = std::env::temp_dir().join(format!("mock-lila-test-{}", std::process::id()));
        let start = fixture(&dir);
        let data_end = days_from_civil(2026, 11, 1) as u64 * DAY;
        let shift = data_end - days_from_civil(2026, 10, 1) as u64 * DAY;
        // The clock stops in the middle of the game of the last day.
        let last = start + 29 * DAY + shift;
        let mock = MockLila::start(MockConfig {
            fixture: dir.clone(),
            data_end,
            clock_start: Some(last + 5_000),
            clock_rate: 0.0,
            max_streams: 1,
            casual: 0.0,
            ..MockConfig::default()
        })
        .await
        .unwrap();
        let url = mock.url();
        let (status, games) = lines(&format!("{url}/api/games/user/ALICE")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(games.len(), 29, "ongoing game hidden");
        assert_eq!(games[0]["createdAt"], start + shift);
        assert_eq!(games[0]["rated"], true);
        assert_eq!(games[0]["players"]["white"]["user"]["id"], "alice");
        assert_eq!(games[0]["moves"], "e4 e5 Nf3 Nc6");

        let (_, games) = lines(&format!("{url}/api/games/user/alice?ongoing=true")).await;
        assert_eq!(games.len(), 30);
        assert_eq!(games[29]["status"], "started");
        assert_eq!(games[29]["moves"], "");

        let since = start + shift + 10 * DAY;
        let until = start + shift + 12 * DAY;
        let (_, games) = lines(&format!(
            "{url}/api/games/user/alice?since={since}&until={until}"
        ))
        .await;
        assert_eq!(games.len(), 3);
        let (_, games) = lines(&format!("{url}/api/games/user/alice?sort=dateDesc&max=2")).await;
        assert_eq!(games.len(), 2);
        assert_eq!(games[0]["createdAt"], start + shift + 28 * DAY);

        let (status, _) = lines(&format!("{url}/api/games/user/carol")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let stats = mock.stats();
        assert_eq!(stats.violations, 0);
        assert_eq!(stats.not_found, 1);
        assert_eq!(stats.max_active_streams, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_rate_limit() {
        let dir = std::env::temp_dir().join(format!("mock-lila-rate-{}", std::process::id()));
        fixture(&dir);
        let mock = MockLila::start(MockConfig {
            fixture: dir.clone(),
            data_end: days_from_civil(2026, 11, 1) as u64 * DAY,
            games_per_second: 100.0,
            max_streams: 1,
            ..MockConfig::default()
        })
        .await
        .unwrap();
        // Two at once are a violation, and closing one is noticed even
        // while the stream waits for the rate limit.
        let url = format!("{}/api/games/user/alice", mock.url());
        let a = reqwest::get(&url).await.unwrap();
        let b = reqwest::get(&url).await.unwrap();
        assert_eq!(mock.stats().violations, 1);
        drop((a, b));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(mock.stats().active_streams, 0);
        let started = Instant::now();
        let (_, games) = lines(&format!("{}/api/games/user/alice", mock.url())).await;
        assert_eq!(games.len(), 30);
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(280), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_months() {
        let ms = days_from_civil(2026, 9, 17) as u64 * 86_400_000 + 1234;
        assert_eq!(
            next_month_start(ms),
            days_from_civil(2026, 10, 1) as u64 * 86_400_000
        );
        let ms = days_from_civil(2025, 12, 31) as u64 * 86_400_000;
        assert_eq!(
            next_month_start(ms),
            days_from_civil(2026, 1, 1) as u64 * 86_400_000
        );
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2));
    }
}
