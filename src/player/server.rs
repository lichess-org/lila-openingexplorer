use std::{
    collections::BTreeMap,
    convert::Infallible,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path, Query, State, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use shakmaty::{Color, EnPassantMode, Position, fen::Fen, san::SanPlus, variant::VariantPosition};
use tokio::{sync::Semaphore, task};

use crate::{
    packed::SubStats,
    player::{
        InvalidUserName, MAX_PLY, PlayerDb, UserId,
        db::WARM_CF,
        explore::{PlayerGameResponse, PlayerGameWithUci, PlayerMove, PlayerResponse, prepare},
        indexer::Indexer,
        keys::PlayerGameIndex,
        query::PlayerQuery,
        queue::{Progress, Ticket},
    },
    server::{
        listen,
        openings::{self, Opening, Openings},
        query::PlayError,
        response::Stats,
    },
    util::{Counter, Exposition, Histogram, micros_to_seconds},
};

#[derive(Debug, Clone)]
pub struct PlayerServerConfig {
    pub bind: SocketAddr,
    pub admin_bind: Option<SocketAddr>,
    pub cors: bool,
    /// Concurrent database reads.
    pub blocking_threads: usize,
    pub openings_dir: Option<PathBuf>,
    pub no_openings: bool,
    /// Snapshots at most this often while indexing.
    pub debounce: Duration,
    /// An empty line after this long without a snapshot.
    pub keep_alive: Duration,
}

impl Default for PlayerServerConfig {
    fn default() -> PlayerServerConfig {
        PlayerServerConfig {
            bind: "127.0.0.1:9003".parse().expect("address"),
            admin_bind: None,
            cors: false,
            blocking_threads: 4,
            openings_dir: None,
            no_openings: false,
            debounce: Duration::from_secs(1),
            keep_alive: Duration::from_secs(8),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bad request: {0}")]
    Query(#[from] QueryRejection),
    #[error("bad request: {0}")]
    Play(#[from] PlayError),
    #[error("bad request: {0}")]
    User(#[from] InvalidUserName),
    #[error("indexing queue is full")]
    QueueFull,
    #[error("not found")]
    NotFound,
    #[error("database: {0}")]
    Db(#[from] crate::player::PlayerDbError),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Error::Query(_) | Error::Play(_) | Error::User(_) => StatusCode::BAD_REQUEST,
            Error::QueueFull => StatusCode::SERVICE_UNAVAILABLE,
            Error::NotFound => StatusCode::NOT_FOUND,
            Error::Db(_) => {
                log::error!("{self}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

/// Ply buckets of requests, by tens up to `MAX_PLY` (where indexed
/// positions end) and more.
const PLY_BUCKETS: usize = MAX_PLY / 10 + 1;

#[derive(Default)]
pub struct ServerMetrics {
    pub requests: Counter,
    pub bad_requests: Counter,
    pub queue_full: Counter,
    /// Streams that ended (final snapshot sent).
    pub streams_complete: Counter,
    /// Streams that the client closed before the final snapshot.
    pub streams_gone: Counter,
    pub stream_micros: Counter,
    /// Snapshots that repeated the last read with a new queue position.
    pub snapshots_repeated: Counter,
    pub keep_alives: Counter,
    /// Database reads of snapshots (on the blocking pool).
    pub read: Histogram,
    /// Snapshots by ply of the position: partial, final.
    pub ply: [(Counter, Counter); PLY_BUCKETS],
    pub warm_hits: Counter,
    pub cold_candidates: Counter,
    pub cold_validated: Counter,
    pub players_deleted: Counter,
}

impl ServerMetrics {
    fn write(&self, out: &mut Exposition) {
        out.counter(
            "requests_total",
            "Valid player requests.",
            self.requests.get(),
        );
        out.counter(
            "bad_requests_total",
            "Rejected player requests.",
            self.bad_requests.get(),
        );
        out.counter(
            "queue_full_total",
            "Requests rejected because the indexing queue was full.",
            self.queue_full.get(),
        );
        out.labeled_counter(
            "streams_ended_total",
            "Response streams that ended, by how: final snapshot sent, or closed by the client before.",
            "end",
            [
                ("complete", self.streams_complete.get()),
                ("gone", self.streams_gone.get()),
            ],
        );
        out.counter(
            "stream_seconds_total",
            "Time that ended response streams were open.",
            micros_to_seconds(self.stream_micros.get()),
        );
        let plies: Vec<String> = (0..PLY_BUCKETS)
            .map(|i| {
                if i + 1 == PLY_BUCKETS {
                    format!("{}+", i * 10)
                } else {
                    format!("{}-{}", i * 10, i * 10 + 9)
                }
            })
            .collect();
        let labels: Vec<[(&str, &str); 2]> = plies
            .iter()
            .flat_map(|ply| {
                [
                    [("ply", ply.as_str()), ("final", "false")],
                    [("ply", ply.as_str()), ("final", "true")],
                ]
            })
            .collect();
        out.family(
            "counter",
            "snapshots_total",
            "Snapshots sent, by ply of the position and whether final.",
            labels.iter().map(|labels| labels.as_slice()).zip(
                self.ply
                    .iter()
                    .flat_map(|(partial, complete)| [partial.get(), complete.get()]),
            ),
        );
        out.counter(
            "snapshots_repeated_total",
            "Snapshots that repeated the last read with a new queue position.",
            self.snapshots_repeated.get(),
        );
        out.counter(
            "keep_alives_total",
            "Keep-alive lines sent.",
            self.keep_alives.get(),
        );
        out.histogram(
            "read_duration_seconds",
            "Database reads of snapshots (on the blocking pool).",
            &self.read,
        );
        out.counter(
            "warm_hits_total",
            "Positions found in the warm column family.",
            self.warm_hits.get(),
        );
        out.counter(
            "cold_candidates_total",
            "Cold candidates of positions.",
            self.cold_candidates.get(),
        );
        out.counter(
            "cold_validated_total",
            "Cold candidates validated by replay.",
            self.cold_validated.get(),
        );
        out.counter(
            "players_deleted_total",
            "Players deleted (DELETE /player/{id}).",
            self.players_deleted.get(),
        );
    }
}

pub struct PlayerState {
    config: PlayerServerConfig,
    indexer: Arc<Indexer>,
    openings: RwLock<Openings>,
    semaphore: Semaphore,
    pub metrics: ServerMetrics,
    open_streams: AtomicU64,
}

type Shared = Arc<PlayerState>;

fn ply(pos: &VariantPosition) -> usize {
    (pos.fullmoves().get() as usize - 1) * 2 + usize::from(pos.turn() == Color::Black)
}

struct Request {
    query: PlayerQuery,
    pos: VariantPosition,
    opening: Option<Opening>,
}

impl PlayerState {
    fn read(&self, request: &Request) -> Result<PlayerResponse, Error> {
        let started = Instant::now();
        let query = &request.query;
        let db = self.indexer.db();
        let uh = db.user_hash(&query.player);
        let reader = db.reader();
        let empty = || PlayerResponse {
            opening: request.opening.clone(),
            queue_position: 0,
            total: Stats::default(),
            moves: Vec::new(),
            recent_games: Vec::new(),
        };
        let Some(status) = reader.status(uh)? else {
            return Ok(empty());
        };
        let (mut entry, stats) = reader.position(&status, query.color, &request.pos)?;
        let m = &self.metrics;
        if stats.warm_hit {
            m.warm_hits.inc();
        }
        m.cold_candidates.add(stats.cold_candidates);
        m.cold_validated.add(stats.cold_validated);
        let mut buf = Vec::new();
        entry.pack_entire(&mut buf);
        let prepared = prepare(&buf, &query.filter, &query.limits);

        // Load the referenced games.
        let mut indexes: Vec<PlayerGameIndex> =
            prepared.moves.iter().filter_map(|m| m.game).collect();
        indexes.extend(prepared.recent_games.iter().map(|&(_, index)| index));
        indexes.sort_unstable();
        indexes.dedup();
        let games = reader.games(uh, &indexes)?;
        let game = |index: PlayerGameIndex| -> Option<PlayerGameResponse> {
            let i = indexes.binary_search(&index).ok()?;
            games[i]
                .as_ref()
                .map(|game| PlayerGameResponse::new(game, &status.name))
        };

        let openings = self.openings.read().expect("read openings");
        let pos = &request.pos;
        let moves = prepared
            .moves
            .iter()
            .filter_map(|pm| {
                let uci = pm.m.unpack();
                let Ok(m) = uci.to_move(pos) else {
                    log::warn!(
                        "skipping illegal move {uci} in {}",
                        Fen::from_position(pos, EnPassantMode::Legal)
                    );
                    return None;
                };
                let mut after = pos.clone();
                let san = SanPlus::from_move_and_play_unchecked(&mut after, m);
                Some(PlayerMove {
                    uci,
                    san,
                    average_opponent_rating: pm.stats.average_rating().map(i32::from),
                    performance: pm.stats.performance(query.color),
                    stats: Stats::from(&pm.stats),
                    game: pm.game.and_then(game),
                    opening: openings.classify_exact(&after).cloned(),
                })
            })
            .collect();
        let recent_games = prepared
            .recent_games
            .iter()
            .filter_map(|&(m, index)| {
                game(index).map(|game| PlayerGameWithUci {
                    uci: m.unpack(),
                    game,
                })
            })
            .collect();
        let total: &SubStats = &prepared.total;
        let response = PlayerResponse {
            opening: request.opening.clone(),
            queue_position: 0,
            total: Stats::from(total),
            moves,
            recent_games,
        };
        m.read.record(started.elapsed());
        Ok(response)
    }

    fn record_ply(&self, request: &Request, done: bool) {
        let bucket = (ply(&request.pos) / 10).min(PLY_BUCKETS - 1);
        let (partial, complete) = &self.metrics.ply[bucket];
        if done { complete } else { partial }.inc();
    }
}

async fn blocking<T: Send + 'static>(
    state: &Shared,
    f: impl FnOnce(&Shared) -> T + Send + 'static,
) -> T {
    let _permit = state
        .semaphore
        .acquire()
        .await
        .expect("semaphore not closed");
    let state = Arc::clone(state);
    task::spawn_blocking(move || f(&state))
        .await
        .expect("blocking task")
}

fn line(response: &PlayerResponse) -> Bytes {
    let mut buf = serde_json::to_vec(response).expect("serialize");
    buf.push(b'\n');
    Bytes::from(buf)
}

/// Counts an open stream, and how it ended.
struct StreamGuard {
    state: Shared,
    started: Instant,
    complete: bool,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let m = &self.state.metrics;
        if self.complete {
            m.streams_complete.inc();
        } else {
            m.streams_gone.inc();
        }
        m.stream_micros
            .add(u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX));
        self.state.open_streams.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Streams the snapshots of a request (first immediately,
/// then at most one per `debounce` while indexing, each re-reading only
/// after committed batches, and the final one immediately).
async fn snapshots(
    state: Shared,
    request: Arc<Request>,
    mut ticket: Ticket,
    tx: tokio::sync::mpsc::Sender<Result<Bytes, Infallible>>,
) {
    state.open_streams.fetch_add(1, Ordering::Relaxed);
    let mut guard = StreamGuard {
        state: Arc::clone(&state),
        started: Instant::now(),
        complete: false,
    };
    let debounce = state.config.debounce;
    let keep_alive = state.config.keep_alive;
    let mut progress = ticket.progress();
    let mut read_generation = progress.generation;
    let read = {
        let request = Arc::clone(&request);
        blocking(&state, move |state| state.read(&request)).await
    };
    let mut last = match read {
        Ok(response) => response,
        Err(err) => {
            log::error!("player snapshot: {err}");
            return;
        }
    };
    let mut last_key = None;
    let mut last_snapshot = Instant::now();
    let mut last_line = Instant::now();
    loop {
        last.queue_position = progress.queue_position;
        // Consecutive snapshots with the same queue position and total are
        // dropped, as before (also an unchanged final snapshot).
        let key = (last.queue_position, last.total_games());
        if last_key != Some(key) {
            state.record_ply(&request, progress.done);
            if tx.send(Ok(line(&last))).await.is_err() {
                return;
            }
            last_key = Some(key);
            last_snapshot = Instant::now();
            last_line = last_snapshot;
        }
        if progress.done {
            guard.complete = true;
            return;
        }

        // Wait for a change, with keep-alive lines meanwhile.
        loop {
            tokio::select! {
                changed = ticket.changed() => {
                    progress = changed;
                    break;
                }
                _ = tokio::time::sleep_until((last_line + keep_alive).into()) => {
                    state.metrics.keep_alives.inc();
                    if tx.send(Ok(Bytes::from_static(b"\n"))).await.is_err() {
                        return;
                    }
                    last_line = Instant::now();
                }
                _ = tx.closed() => return,
            }
        }
        // Debounce, except for the final snapshot.
        if !progress.done {
            tokio::select! {
                _ = tokio::time::sleep_until((last_snapshot + debounce).into()) => (),
                _ = tx.closed() => return,
            }
            progress = merge_progress(progress, ticket.progress());
        }
        if progress.generation != read_generation || progress.done {
            read_generation = progress.generation;
            let request = Arc::clone(&request);
            match blocking(&state, move |state| state.read(&request)).await {
                Ok(response) => last = response,
                Err(err) => {
                    log::error!("player snapshot: {err}");
                    return;
                }
            }
        } else {
            state.metrics.snapshots_repeated.inc();
        }
    }
}

/// The latest progress (a dropped ticket reports done).
fn merge_progress(waited: Progress, now: Progress) -> Progress {
    Progress {
        done: waited.done || now.done,
        ..now
    }
}

async fn player(
    State(state): State<Shared>,
    query: Result<Query<PlayerQuery>, QueryRejection>,
) -> Result<Response, Error> {
    let query = match query {
        Ok(Query(query)) => query,
        Err(err) => {
            state.metrics.bad_requests.inc();
            return Err(err.into());
        }
    };
    let play = query
        .play
        .position(&state.openings.read().expect("read openings"))
        .inspect_err(|_| state.metrics.bad_requests.inc())?;
    state.metrics.requests.inc();
    let ticket = state.indexer.request(&query.player).await.map_err(|_| {
        state.metrics.queue_full.inc();
        log::error!("not indexing {} because the queue is full", query.player);
        Error::QueueFull
    })?;
    let request = Arc::new(Request {
        query,
        pos: play.pos,
        opening: play.opening,
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    task::spawn(snapshots(Arc::clone(&state), request, ticket, tx));
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(futures_util::stream::poll_fn(
            move |cx| rx.poll_recv(cx),
        )))
        .expect("response"))
}

async fn metrics(State(state): State<Shared>) -> Response {
    use rocksdb::statistics::Ticker;
    let mut out = Exposition::new();
    state.metrics.write(&mut out);
    out.gauge(
        "open_streams",
        "Open response streams.",
        state.open_streams.load(Ordering::Relaxed),
    );
    let indexer = &state.indexer;
    let queue = indexer.queue().stats();
    out.gauge(
        "queue_waiting",
        "Players waiting in the indexing queue.",
        queue.waiting,
    );
    out.gauge("queue_running", "Index runs in progress.", queue.running);
    out.gauge(
        "queue_busy",
        "Players that are busy (queued, being indexed or written).",
        queue.busy,
    );
    out.gauge(
        "lila_streams",
        "Open game streams from lila.",
        indexer.lila().busy(),
    );
    indexer.metrics.write(&mut out);
    let db = indexer.db();
    db.counters.write(&mut out);
    out.gauge(
        "block_cache_usage_bytes",
        "Memory used by the RocksDB block cache.",
        db.cache_usage(),
    );
    for (prop, name, help) in [
        (
            "rocksdb.estimate-num-keys",
            "rocksdb_estimated_keys",
            "Estimated keys of the column family.",
        ),
        (
            "rocksdb.total-sst-files-size",
            "rocksdb_sst_files_bytes",
            "Size of the SST files of the column family.",
        ),
        (
            "rocksdb.estimate-pending-compaction-bytes",
            "rocksdb_pending_compaction_bytes",
            "Estimated bytes that compactions of the column family need to rewrite.",
        ),
    ] {
        out.labeled_gauge(
            name,
            help,
            "cf",
            PlayerDb::column_families()
                .into_iter()
                .filter_map(|cf| Some((cf, db.cf_int_property(cf, prop).ok().flatten()?))),
        );
    }
    write_levels(db, &mut out);
    for (prop, name, help) in [
        (
            "rocksdb.actual-delayed-write-rate",
            "rocksdb_delayed_write_rate_bytes_per_second",
            "Rate of delayed writes (0 if not delayed).",
        ),
        (
            "rocksdb.is-write-stopped",
            "rocksdb_write_stopped",
            "1 if writes are stopped.",
        ),
        (
            "rocksdb.num-running-compactions",
            "rocksdb_running_compactions",
            "Compactions in progress.",
        ),
        (
            "rocksdb.num-running-flushes",
            "rocksdb_running_flushes",
            "Flushes in progress.",
        ),
    ] {
        if let Ok(Some(value)) = db.cf_int_property(WARM_CF, prop) {
            out.gauge(name, help, value);
        }
    }
    if let Some(micros) = db.ticker(Ticker::StallMicros) {
        out.counter(
            "rocksdb_stall_seconds_total",
            "Time that writes were stalled.",
            micros_to_seconds(micros),
        );
    }
    for (ticker, name, help) in [
        (
            Ticker::CompactReadBytes,
            "rocksdb_compaction_read_bytes_total",
            "Bytes read by compactions.",
        ),
        (
            Ticker::CompactWriteBytes,
            "rocksdb_compaction_write_bytes_total",
            "Bytes written by compactions.",
        ),
        (
            Ticker::FlushWriteBytes,
            "rocksdb_flush_write_bytes_total",
            "Bytes written by flushes.",
        ),
        (
            Ticker::BlockCacheHit,
            "rocksdb_block_cache_hits_total",
            "RocksDB block cache hits.",
        ),
    ] {
        if let Some(value) = db.ticker(ticker) {
            out.counter(name, help, value);
        }
    }
    for (ticker, name, help) in [
        (
            Ticker::BytesWritten,
            "rocksdb_bytes_written_total",
            "Bytes of user writes (keys, values and merge operands), the base of write amplification.",
        ),
        (
            Ticker::WalFileBytes,
            "rocksdb_wal_write_bytes_total",
            "Bytes written to the write ahead log.",
        ),
        (
            Ticker::BloomFilterUseful,
            "rocksdb_bloom_filter_useful_total",
            "Point lookups in SST files that a filter ruled out.",
        ),
        (
            Ticker::BloomFilterFullPositive,
            "rocksdb_bloom_filter_positive_total",
            "Point lookups in SST files that a filter did not rule out.",
        ),
        (
            Ticker::BloomFilterFullTruePositive,
            "rocksdb_bloom_filter_true_positive_total",
            "Point lookups in SST files that a filter did not rule out, and that found the key.",
        ),
    ] {
        if let Some(value) = db.ticker(ticker) {
            out.counter(name, help, value);
        }
    }
    out.labeled_counter(
        "rocksdb_get_hits_total",
        "Point lookups that found the key, by where.",
        "source",
        [
            ("memtable", Ticker::MemtableHit),
            ("L0", Ticker::GetHitL0),
            ("L1", Ticker::GetHitL1),
            ("L2+", Ticker::GetHitL2AndUp),
        ]
        .into_iter()
        .filter_map(|(source, ticker)| Some((source, db.ticker(ticker)?))),
    );
    out.labeled_counter(
        "rocksdb_file_reads_total",
        "Reads from SST files (by queries and compactions), by whether from the last level (on the last --db-path).",
        "level",
        [
            ("last", Ticker::LastLevelReadCount),
            ("non_last", Ticker::NonLastLevelReadCount),
        ]
        .into_iter()
        .filter_map(|(level, ticker)| Some((level, db.ticker(ticker)?))),
    );
    out.labeled_counter(
        "rocksdb_file_read_bytes_total",
        "Bytes read from SST files (by queries and compactions), by whether from the last level (on the last --db-path).",
        "level",
        [
            ("last", Ticker::LastLevelReadBytes),
            ("non_last", Ticker::NonLastLevelReadBytes),
        ]
        .into_iter()
        .filter_map(|(level, ticker)| Some((level, db.ticker(ticker)?))),
    );
    out.labeled_counter(
        "rocksdb_block_cache_misses_total",
        "RocksDB block cache misses, by block type.",
        "block",
        [
            ("index", Ticker::BlockCacheIndexMiss),
            ("filter", Ticker::BlockCacheFilterMiss),
            ("data", Ticker::BlockCacheDataMiss),
        ]
        .into_iter()
        .filter_map(|(block, ticker)| Some((block, db.ticker(ticker)?))),
    );
    out.gauge(
        "openings",
        "Known opening names.",
        state.openings.read().expect("read openings").len(),
    );
    listen::metrics_response(out)
}

/// Sizes of the levels of all column families, and the estimated live
/// data (for space amplification).
fn write_levels(db: &PlayerDb, out: &mut Exposition) {
    out.labeled_gauge(
        "rocksdb_estimated_live_data_bytes",
        "Estimated size of the live data of the column family (compare with rocksdb_sst_files_bytes for space amplification).",
        "cf",
        PlayerDb::column_families().into_iter().filter_map(|cf| {
            Some((
                cf,
                db.cf_int_property(cf, "rocksdb.estimate-live-data-size")
                    .ok()
                    .flatten()?,
            ))
        }),
    );
    let files = match db.live_files() {
        Ok(files) => files,
        Err(err) => {
            log::error!("live files for /metrics: {err}");
            return;
        }
    };
    #[derive(Default)]
    struct Level {
        files: u64,
        bytes: u64,
        entries: u64,
        deletions: u64,
    }
    let mut levels: BTreeMap<(&str, i32), Level> = BTreeMap::new();
    for file in &files {
        let level = levels
            .entry((file.column_family_name.as_str(), file.level))
            .or_default();
        level.files += 1;
        level.bytes += file.size as u64;
        level.entries += file.num_entries;
        level.deletions += file.num_deletions;
    }
    let labels: Vec<(&str, String)> = levels
        .keys()
        .map(|&(cf, level)| (cf, level.to_string()))
        .collect();
    let labels: Vec<[(&str, &str); 2]> = labels
        .iter()
        .map(|(cf, level)| [("cf", *cf), ("level", level.as_str())])
        .collect();
    for (name, help, value) in [
        (
            "rocksdb_level_files",
            "SST files of the level.",
            (|level: &Level| level.files) as fn(&Level) -> u64,
        ),
        (
            "rocksdb_level_bytes",
            "Size of the SST files of the level.",
            |level: &Level| level.bytes,
        ),
        (
            "rocksdb_level_entries",
            "Entries in the SST files of the level (including deletions and merge operands).",
            |level: &Level| level.entries,
        ),
        (
            "rocksdb_level_deletions",
            "Deletion entries in the SST files of the level.",
            |level: &Level| level.deletions,
        ),
    ] {
        out.family(
            "gauge",
            name,
            help,
            labels
                .iter()
                .map(|labels| labels.as_slice())
                .zip(levels.values().map(value)),
        );
    }
}

async fn delete_player(
    State(state): State<Shared>,
    Path(user): Path<String>,
) -> Result<StatusCode, Error> {
    let user = UserId::from_name(&user)?;
    let existed = state.indexer.delete(&user).await?;
    Ok(if existed {
        state.metrics.players_deleted.inc();
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn db_prop(State(state): State<Shared>, Path(prop): Path<String>) -> Result<String, Error> {
    blocking(&state, move |state| {
        state
            .indexer
            .db()
            .property(&prop)
            .ok()
            .flatten()
            .ok_or(Error::NotFound)
    })
    .await
}

async fn cf_prop(
    State(state): State<Shared>,
    Path((cf, prop)): Path<(String, String)>,
) -> Result<String, Error> {
    blocking(&state, move |state| {
        state
            .indexer
            .db()
            .cf_property(&cf, &prop)
            .ok()
            .flatten()
            .ok_or(Error::NotFound)
    })
    .await
}

pub fn router(
    config: PlayerServerConfig,
    indexer: Arc<Indexer>,
    openings: Openings,
) -> (Router, Router, Shared) {
    let state: Shared = Arc::new(PlayerState {
        semaphore: Semaphore::new(config.blocking_threads),
        openings: RwLock::new(openings),
        metrics: ServerMetrics::default(),
        open_streams: AtomicU64::new(0),
        config,
        indexer,
    });
    let app = Router::new()
        .route("/player", get(player))
        .with_state(Arc::clone(&state));
    let admin = Router::new()
        .route("/metrics", get(metrics))
        .route("/db/{prop}", get(db_prop))
        .route("/cf/{cf}/{prop}", get(cf_prop))
        .route("/player/{id}", delete(delete_player))
        .with_state(Arc::clone(&state));
    let app = if state.config.cors {
        app.layer(listen::cors())
    } else {
        app
    };
    (app, admin, state)
}

pub async fn serve(
    config: PlayerServerConfig,
    indexer: Arc<Indexer>,
) -> Result<(), Box<dyn std::error::Error>> {
    let openings = Openings::initial(config.openings_dir.as_deref())?;
    let download = config.openings_dir.is_none() && !config.no_openings;
    let (bind, admin_bind) = (config.bind, config.admin_bind);
    let (app, admin, state) = router(config, indexer, openings);
    if download {
        let state = Arc::clone(&state);
        task::spawn(async move { openings::periodic_refresh(&state.openings, || ()).await });
    }
    listen::serve_until_signal(bind, app, admin_bind.map(|addr| (addr, admin))).await?;
    Ok(())
}
