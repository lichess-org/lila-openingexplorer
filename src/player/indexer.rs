use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{
    sync::{Semaphore, mpsc},
    task,
};

use crate::{
    player::{
        PlayerDb, PlayerDbError, UserId,
        game::PlayerGame,
        lila::{Classified, ExportGame, LilaClient, LilaError, Skip},
        policy::{IndexRun, RunPolicy},
        queue::{ProgressHandle, Queue, QueueFull, QueueItem, Ticket},
        status::PlayerStatus,
        time::UnixMillis,
    },
    util::{Counter, Exposition, micros_to_seconds},
};

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Parallel index runs (each with one stream from lila).
    pub indexers: usize,
    /// Players waiting at most.
    pub queue_capacity: usize,
    /// Queue players that have an index (incremental runs) before first
    /// runs.
    pub known_first: bool,
    pub policy: RunPolicy,
    /// Games per chunk at most.
    pub chunk_games: usize,
    /// A chunk is written at the latest after this time, so that waiting
    /// requests see progress.
    pub chunk_interval: Duration,
    /// Index runs give up their stream after this time and continue at the
    /// end of the queue (`None` to run until all games are indexed).
    pub max_run: Option<Duration>,
    /// Rebuild a player after `max(rebuild_min_games, rebuild_factor *
    /// games at the last rebuild)` new games.
    pub rebuild_min_games: u32,
    pub rebuild_factor: f64,
    /// Parallel rebuilds at most.
    pub rebuilds: usize,
    /// Retries of a failed request or stream in a row (without games
    /// received between), in the same slot.
    pub retries: u32,
    /// Delay before the first retry, growing linearly with each failure in
    /// a row.
    pub retry_delay: Duration,
}

impl Default for IndexerConfig {
    fn default() -> IndexerConfig {
        IndexerConfig {
            indexers: 8,
            queue_capacity: 2000,
            known_first: true,
            policy: RunPolicy::default(),
            chunk_games: 2000,
            chunk_interval: Duration::from_secs(1),
            max_run: Some(Duration::from_secs(30)),
            rebuild_min_games: 1000,
            rebuild_factor: 0.5,
            rebuilds: 1,
            retries: 3,
            retry_delay: Duration::from_secs(5),
        }
    }
}

/// Counters of the indexers, for `/metrics`.
#[derive(Default)]
pub struct IndexerMetrics {
    pub runs: Counter,
    pub runs_complete: Counter,
    pub runs_not_found: Counter,
    pub runs_failed: Counter,
    pub runs_abandoned: Counter,
    pub runs_requeued: Counter,
    pub games_received: Counter,
    pub skipped_ongoing: Counter,
    pub skipped_unfinished: Counter,
    pub skipped_anonymous: Counter,
    pub skipped_invalid: Counter,
    pub malformed_lines: Counter,
    pub lila_errors: Counter,
    pub lila_timeouts: Counter,
    pub stream_retries: Counter,
    /// Time that streams waited for chunk writes.
    pub writer_wait_micros: Counter,
    /// Time that streams were open.
    pub stream_micros: Counter,
    pub rebuild_errors: Counter,
    pub queue_full: Counter,
}

impl IndexerMetrics {
    pub fn write(&self, out: &mut Exposition) {
        out.counter(
            "index_runs_started_total",
            "Index runs started.",
            self.runs.get(),
        );
        out.labeled_counter(
            "index_runs_ended_total",
            "Index runs that ended, by how.",
            "end",
            [
                ("complete", self.runs_complete.get()),
                ("not_found", self.runs_not_found.get()),
                ("failed", self.runs_failed.get()),
                ("abandoned", self.runs_abandoned.get()),
                ("requeued", self.runs_requeued.get()),
            ],
        );
        out.counter(
            "games_received_total",
            "Games received from lila.",
            self.games_received.get(),
        );
        out.labeled_counter(
            "games_skipped_total",
            "Received games that were not indexed, by reason.",
            "reason",
            [
                ("ongoing", self.skipped_ongoing.get()),
                ("unfinished", self.skipped_unfinished.get()),
                ("anonymous", self.skipped_anonymous.get()),
                ("invalid", self.skipped_invalid.get()),
            ],
        );
        out.counter(
            "lila_malformed_lines_total",
            "Malformed lines in game streams from lila.",
            self.malformed_lines.get(),
        );
        out.counter(
            "lila_errors_total",
            "Errors of game streams from lila.",
            self.lila_errors.get(),
        );
        out.counter(
            "lila_timeouts_total",
            "Timeouts of game streams from lila.",
            self.lila_timeouts.get(),
        );
        out.counter(
            "lila_stream_retries_total",
            "Retried game streams from lila.",
            self.stream_retries.get(),
        );
        out.counter(
            "writer_wait_seconds_total",
            "Time that game streams waited for chunk writes.",
            micros_to_seconds(self.writer_wait_micros.get()),
        );
        out.counter(
            "lila_stream_seconds_total",
            "Time that game streams from lila were open.",
            micros_to_seconds(self.stream_micros.get()),
        );
        out.counter(
            "rebuild_errors_total",
            "Failed rebuilds of player indexes.",
            self.rebuild_errors.get(),
        );
    }
}

/// How an index run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunEnd {
    /// All games received.
    Complete,
    /// Unknown or closed account.
    NotFound,
    /// Too many errors.
    Failed,
    /// No request waits for the player any more.
    Abandoned,
    /// Out of time, to continue later.
    Requeue,
}

/// A chunk of an index run, for the writer.
#[derive(Default)]
struct Chunk {
    games: Vec<PlayerGame>,
    latest_created_at: UnixMillis,
    /// Earliest `createdAt` of the ongoing games of the chunk.
    ongoing: Option<UnixMillis>,
    name: Option<String>,
    /// For an interrupted revisit: where the next revisit continues.
    revisit_from: Option<UnixMillis>,
    finish: Option<Finish>,
}

struct Finish {
    indexed_at: UnixMillis,
    /// For a complete revisit: the new earliest ongoing game.
    revisit: Option<Option<UnixMillis>>,
}

fn min_opt<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub struct Indexer {
    queue: Arc<Queue>,
    db: Arc<PlayerDb>,
    lila: Arc<LilaClient>,
    config: IndexerConfig,
    rebuilds: Arc<Semaphore>,
    pub metrics: IndexerMetrics,
}

impl Indexer {
    /// Starts the indexer tasks.
    pub fn start(db: Arc<PlayerDb>, lila: Arc<LilaClient>, config: IndexerConfig) -> Arc<Indexer> {
        let indexer = Arc::new(Indexer {
            queue: Arc::new(Queue::new(config.queue_capacity, config.known_first)),
            rebuilds: Arc::new(Semaphore::new(config.rebuilds.max(1))),
            db,
            lila,
            config,
            metrics: IndexerMetrics::default(),
        });
        for idx in 0..indexer.config.indexers {
            let indexer = Arc::clone(&indexer);
            task::spawn(async move {
                loop {
                    let item = indexer.queue.acquire().await;
                    indexer.index(idx, item).await;
                }
            });
        }
        indexer
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn lila(&self) -> &LilaClient {
        &self.lila
    }

    pub fn db(&self) -> &Arc<PlayerDb> {
        &self.db
    }

    /// The ticket for a request of the player: queues the player if an index
    /// run is due.
    pub async fn request(&self, user: &UserId) -> Result<Ticket, QueueFull> {
        if let Some(ticket) = self.queue.watch(user) {
            return Ok(ticket);
        }
        let db = Arc::clone(&self.db);
        let uh = db.user_hash(user);
        let status = task::spawn_blocking(move || db.status(uh))
            .await
            .expect("status task");
        let status = match status {
            Ok(status) => status,
            Err(err) => {
                log::error!("reading status of {user}: {err}");
                return Ok(Ticket::completed());
            }
        };
        if self
            .config
            .policy
            .next_run(status.as_ref(), UnixMillis::now())
            .is_none()
        {
            return Ok(Ticket::completed());
        }
        let known = status.is_some_and(|status| status.games > 0);
        self.queue.submit(user.clone(), known).inspect_err(|_| {
            self.metrics.queue_full.inc();
        })
    }

    /// Deletes all data of the player, after waiting for a running index
    /// run or rebuild of the player to end. Requests waiting for the player
    /// get a new first run afterwards. Returns whether the player was
    /// indexed.
    pub async fn delete(&self, user: &UserId) -> Result<bool, PlayerDbError> {
        let _busy = self.queue.reserve(user).await;
        let db = Arc::clone(&self.db);
        let uh = db.user_hash(user);
        let existed = task::spawn_blocking(move || -> Result<bool, PlayerDbError> {
            let existed = db.status(uh)?.is_some();
            db.delete_player(uh)?;
            Ok(existed)
        })
        .await
        .expect("delete task")?;
        log::info!(
            "deleted {user}{}",
            if existed { "" } else { " (was not indexed)" }
        );
        Ok(existed)
    }

    async fn index(self: &Arc<Self>, idx: usize, item: QueueItem) {
        let user = item.user().clone();
        let uh = self.db.user_hash(&user);
        let db = Arc::clone(&self.db);
        let status = match task::spawn_blocking(move || db.status(uh))
            .await
            .expect("status task")
        {
            Ok(status) => status,
            Err(err) => {
                log::error!("indexer {idx:02}: reading status of {user}: {err}");
                return;
            }
        };
        let Some(run) = self
            .config
            .policy
            .next_run(status.as_ref(), UnixMillis::now())
        else {
            // Indexed meanwhile.
            return;
        };
        let status = status.unwrap_or_else(|| PlayerStatus::new(user.clone()));
        let games_before = status.games;
        self.metrics.runs.inc();
        let started = Instant::now();
        log::debug!(
            "indexer {idx:02}: starting {user} since {}{}",
            run.since,
            if run.revisit { " (revisit)" } else { "" }
        );

        let (tx, rx) = mpsc::channel::<Chunk>(2);
        let writer = {
            let db = Arc::clone(&self.db);
            let run = run.clone();
            let progress = item.progress();
            task::spawn_blocking(move || write_chunks(&db, status, run, rx, progress))
        };
        let end = self.stream(idx, &item, &user, run, &tx).await;
        drop(tx);
        let status = writer.await.expect("writer task");

        let counter = match end {
            RunEnd::Complete => &self.metrics.runs_complete,
            RunEnd::NotFound => &self.metrics.runs_not_found,
            RunEnd::Failed => &self.metrics.runs_failed,
            RunEnd::Abandoned => &self.metrics.runs_abandoned,
            RunEnd::Requeue => &self.metrics.runs_requeued,
        };
        counter.inc();
        let Some(status) = status else {
            return;
        };
        let new_games = status.games - games_before;
        let elapsed = started.elapsed();
        if new_games > 0 || end != RunEnd::Complete {
            log::info!(
                "indexer {idx:02}: {user}: {new_games} new games in {elapsed:.3?} ({:.0} games/s), {end:?}",
                f64::from(new_games) / elapsed.as_secs_f64()
            );
        }

        if end == RunEnd::Requeue && !item.abandoned() {
            item.requeue();
            return;
        }
        if end != RunEnd::Requeue
            && self.db.rebuild_due(
                &status,
                self.config.rebuild_min_games,
                self.config.rebuild_factor,
            )
        {
            let busy = item.finish_busy();
            let indexer = Arc::clone(self);
            task::spawn(async move {
                let _permit = Arc::clone(&indexer.rebuilds)
                    .acquire_owned()
                    .await
                    .expect("rebuild semaphore");
                let db = Arc::clone(&indexer.db);
                let started = Instant::now();
                match task::spawn_blocking(move || db.rebuild(uh))
                    .await
                    .expect("rebuild task")
                {
                    Ok(Some(stats)) => log::info!(
                        "rebuilt {user} in {:.3?}: {} games, {} warm, {} cold groups, {} bytes{}",
                        started.elapsed(),
                        stats.games,
                        stats.warm_entries,
                        stats.cold_groups,
                        stats.bytes,
                        if stats.ingested { ", ingested" } else { "" }
                    ),
                    Ok(None) => (),
                    Err(err) => {
                        indexer.metrics.rebuild_errors.inc();
                        log::error!("rebuilding {user}: {err}");
                    }
                }
                drop(busy);
            });
        }
    }

    /// Streams the games of the run from lila into chunks for the writer.
    async fn stream(
        &self,
        idx: usize,
        item: &QueueItem,
        user: &UserId,
        run: IndexRun,
        tx: &mpsc::Sender<Chunk>,
    ) -> RunEnd {
        let mut slot = self.lila.slot().await;
        let started = Instant::now();
        let mut since = run.since;
        let mut failures = 0;
        let mut chunk = Chunk::default();
        let mut last_flush = Instant::now();
        // Ongoing games of this run, for the revisit pointer.
        let mut ongoing: Option<UnixMillis> = None;
        let m = &self.metrics;

        let flush = async |chunk: &mut Chunk, finish: Option<Finish>| {
            let chunk = Chunk {
                finish,
                ..std::mem::take(chunk)
            };
            let waited = Instant::now();
            let ok = tx.send(chunk).await.is_ok();
            m.writer_wait_micros.add(micros(waited.elapsed()));
            ok
        };

        let end = 'run: loop {
            let mut stream = match slot.user_games(user, since).await {
                Ok(stream) => stream,
                Err(LilaError::NotFound) => {
                    log::warn!("indexer {idx:02}: did not find player {user}");
                    break 'run RunEnd::NotFound;
                }
                Err(err) => {
                    if matches!(err, LilaError::Timeout) {
                        m.lila_timeouts.inc();
                    } else {
                        m.lila_errors.inc();
                    }
                    log::error!("indexer {idx:02}: request for {user}: {err}");
                    failures += 1;
                    if failures > self.config.retries {
                        break 'run RunEnd::Failed;
                    }
                    m.stream_retries.inc();
                    tokio::time::sleep(self.config.retry_delay * failures).await;
                    continue;
                }
            };
            let mut received = false;
            loop {
                let line = match stream.next_line().await {
                    Ok(Some(line)) => line,
                    Ok(None) => break 'run RunEnd::Complete,
                    Err(err) => {
                        if matches!(err, LilaError::Timeout) {
                            m.lila_timeouts.inc();
                        } else {
                            m.lila_errors.inc();
                        }
                        log::error!("indexer {idx:02}: stream for {user}: {err}");
                        break;
                    }
                };
                let game = match ExportGame::parse(&line) {
                    Ok(game) => game,
                    Err(err) => {
                        m.malformed_lines.inc();
                        log::warn!("indexer {idx:02}: malformed line for {user}: {err}");
                        continue;
                    }
                };
                m.games_received.inc();
                received = true;
                // Retries include the latest millisecond (see `RunPolicy`).
                since = since.max(game.created_at);
                chunk.latest_created_at = chunk.latest_created_at.max(game.created_at);
                match game.classify(user) {
                    Classified::Game { game, name } => {
                        chunk.games.push(*game);
                        chunk.name = Some(name);
                    }
                    Classified::Skip(Skip::Ongoing) => {
                        m.skipped_ongoing.inc();
                        chunk.ongoing = min_opt(chunk.ongoing, Some(game.created_at));
                        ongoing = min_opt(ongoing, Some(game.created_at));
                    }
                    Classified::Skip(Skip::Unfinished) => m.skipped_unfinished.inc(),
                    Classified::Skip(Skip::Anonymous) => m.skipped_anonymous.inc(),
                    Classified::Skip(Skip::NotPlayer | Skip::Invalid) => m.skipped_invalid.inc(),
                }
                if chunk.games.len() >= self.config.chunk_games
                    || last_flush.elapsed() >= self.config.chunk_interval
                {
                    last_flush = Instant::now();
                    if !flush(&mut chunk, None).await {
                        break 'run RunEnd::Failed;
                    }
                    if item.abandoned() {
                        break 'run RunEnd::Abandoned;
                    }
                    if self
                        .config
                        .max_run
                        .is_some_and(|max_run| started.elapsed() >= max_run)
                    {
                        break 'run RunEnd::Requeue;
                    }
                }
            }
            // The stream broke: retry from the checkpoint, in the same slot.
            // Failures count only in a row, without games received between.
            drop(stream);
            failures = if received { 1 } else { failures + 1 };
            if failures > self.config.retries {
                break 'run RunEnd::Failed;
            }
            m.stream_retries.inc();
            tokio::time::sleep(self.config.retry_delay * failures).await;
        };
        drop(slot);
        m.stream_micros.add(micros(started.elapsed()));

        // Finished runs start the cooldown, also after errors (to not
        // retry a failing player at once). Interrupted runs continue from
        // the checkpoint with the next request.
        let finish = match end {
            RunEnd::Complete | RunEnd::NotFound | RunEnd::Failed => Some(Finish {
                indexed_at: UnixMillis::now(),
                revisit: (run.revisit && end == RunEnd::Complete).then_some(ongoing),
            }),
            RunEnd::Abandoned | RunEnd::Requeue => None,
        };
        // An interrupted revisit has checked the games created before
        // `since` (the ongoing ones are in `ongoing`), so that the next one
        // continues from there, instead of starting over (and maybe never
        // completing within `max_run`).
        if run.revisit && end != RunEnd::Complete {
            chunk.revisit_from = Some(revisit_from(ongoing, since));
        }
        flush(&mut chunk, finish).await;
        end
    }
}

/// Where the next revisit continues after an interrupted one, which
/// received the games created before `since` (and before, the ongoing
/// games, the earliest of which is `ongoing`).
fn revisit_from(ongoing: Option<UnixMillis>, since: UnixMillis) -> UnixMillis {
    ongoing.map_or(since, |ongoing| ongoing.min(since))
}

/// Writes the chunks of a run, returning the final status (`None` after a
/// database error, which also ends the stream: its next chunk cannot be
/// sent).
fn write_chunks(
    db: &PlayerDb,
    mut status: PlayerStatus,
    run: IndexRun,
    mut rx: mpsc::Receiver<Chunk>,
    progress: ProgressHandle,
) -> Option<PlayerStatus> {
    while let Some(chunk) = rx.blocking_recv() {
        status.latest_created_at = status.latest_created_at.max(chunk.latest_created_at);
        if !run.revisit {
            status.revisit_ongoing = min_opt(status.revisit_ongoing, chunk.ongoing);
        }
        if let Some(name) = chunk.name {
            status.name = name;
        }
        if let Some(from) = chunk.revisit_from {
            status.revisit_ongoing = Some(from);
        }
        if let Some(finish) = &chunk.finish {
            status.indexed_at = finish.indexed_at;
            if let Some(revisit) = finish.revisit {
                status.revisit_ongoing = revisit;
                status.revisited_at = finish.indexed_at;
            }
        }
        if chunk.games.is_empty()
            && chunk.finish.is_none()
            && chunk.revisit_from.is_none()
            && chunk.latest_created_at == UnixMillis::EPOCH
        {
            continue;
        }
        match db.write_chunk(&mut status, &chunk.games) {
            Ok(stats) => {
                if stats.games > 0 {
                    progress.committed();
                }
            }
            Err(err) => {
                log::error!("writing games of {}: {err}", status.user);
                return None;
            }
        }
    }
    Some(status)
}

#[cfg(test)]
mod tests {
    use arbitrary::Unstructured;
    use shakmaty::{Color, variant::VariantPosition};

    use super::*;
    use crate::{
        mock_lila::{
            Faults, MockConfig, MockLila, Noise,
            fixture::{FixtureGame, write_synthetic},
        },
        player::{
            PlayerDbConfig,
            lila::LilaClientConfig,
            tests::{TempDir, arbitrary_game},
        },
        util::civil::{MS_PER_DAY, days_from_civil},
    };

    fn games(n: u64) -> Vec<PlayerGame> {
        let data = [7; 4096];
        let mut u = Unstructured::new(&data);
        (0..n)
            .map(|i| arbitrary_game(&mut u, i).unwrap().0)
            .collect()
    }

    /// Runs the writer on the chunks.
    fn write(
        db: &PlayerDb,
        status: PlayerStatus,
        run: IndexRun,
        chunks: Vec<Chunk>,
    ) -> (Option<PlayerStatus>, Ticket) {
        let (tx, rx) = mpsc::channel(chunks.len().max(1));
        for chunk in chunks {
            tx.try_send(chunk).expect("channel capacity");
        }
        drop(tx);
        let (progress, ticket) = ProgressHandle::detached();
        (write_chunks(db, status, run, rx, progress), ticket)
    }

    fn setup() -> (TempDir, PlayerDb, PlayerStatus) {
        let dir = TempDir::new("writer");
        let db = PlayerDb::open(&dir.0, PlayerDbConfig::default()).unwrap();
        let status = PlayerStatus::new(UserId::from_name("alice").unwrap());
        (dir, db, status)
    }

    const RUN: IndexRun = IndexRun {
        since: UnixMillis(0),
        revisit: false,
    };

    const REVISIT: IndexRun = IndexRun {
        since: UnixMillis(10),
        revisit: true,
    };

    #[test]
    fn test_run() {
        let (_dir, db, status) = setup();
        let (status, ticket) = write(
            &db,
            status,
            RUN,
            vec![
                Chunk {
                    games: games(2),
                    latest_created_at: UnixMillis(100),
                    ongoing: Some(UnixMillis(60)),
                    name: Some("Alice".to_owned()),
                    ..Chunk::default()
                },
                // Skipped.
                Chunk::default(),
                Chunk {
                    latest_created_at: UnixMillis(120),
                    ongoing: Some(UnixMillis(50)),
                    finish: Some(Finish {
                        indexed_at: UnixMillis(1000),
                        revisit: None,
                    }),
                    ..Chunk::default()
                },
            ],
        );
        let status = status.unwrap();
        assert_eq!(status.games, 2);
        assert_eq!(status.name, "Alice");
        assert_eq!(status.latest_created_at, UnixMillis(120));
        assert_eq!(status.revisit_ongoing, Some(UnixMillis(50)));
        assert_eq!(status.indexed_at, UnixMillis(1000));
        assert_eq!(status.revisited_at, UnixMillis(0));
        assert_eq!(ticket.progress().generation, 1);
        assert_eq!(db.status(db.user_hash(&status.user)).unwrap(), Some(status));
    }

    #[test]
    fn test_revisit() {
        let (_dir, db, mut status) = setup();
        status.revisit_ongoing = Some(UnixMillis(10));
        let (status, _) = write(
            &db,
            status,
            REVISIT,
            vec![Chunk {
                games: games(1),
                latest_created_at: UnixMillis(100),
                // Not the earliest ongoing game, which the run finds.
                ongoing: Some(UnixMillis(5)),
                finish: Some(Finish {
                    indexed_at: UnixMillis(1000),
                    revisit: Some(Some(UnixMillis(40))),
                }),
                ..Chunk::default()
            }],
        );
        let status = status.unwrap();
        assert_eq!(status.revisit_ongoing, Some(UnixMillis(40)));
        assert_eq!(status.revisited_at, UnixMillis(1000));

        // No ongoing games left.
        let (status, _) = write(
            &db,
            status,
            REVISIT,
            vec![Chunk {
                finish: Some(Finish {
                    indexed_at: UnixMillis(2000),
                    revisit: Some(None),
                }),
                ..Chunk::default()
            }],
        );
        let status = status.unwrap();
        assert_eq!(status.revisit_ongoing, None);
        assert_eq!(status.revisited_at, UnixMillis(2000));
    }

    /// An interrupted revisit continues from its checkpoint, and remains
    /// due.
    #[test]
    fn test_interrupted_revisit() {
        let (_dir, db, mut status) = setup();
        status.revisit_ongoing = Some(UnixMillis(10));
        status.revisited_at = UnixMillis(500);
        let (status, _) = write(
            &db,
            status,
            REVISIT,
            vec![Chunk {
                games: games(1),
                latest_created_at: UnixMillis(100),
                revisit_from: Some(revisit_from(Some(UnixMillis(30)), UnixMillis(70))),
                ..Chunk::default()
            }],
        );
        let status = status.unwrap();
        assert_eq!(status.revisit_ongoing, Some(UnixMillis(30)));
        assert_eq!(status.revisited_at, UnixMillis(500));
        assert_eq!(status.indexed_at, UnixMillis(0));
        let policy = RunPolicy::default();
        assert_eq!(
            policy.next_run(
                Some(&status),
                UnixMillis(500 + policy.revisit_interval.as_millis() as u64)
            ),
            Some(IndexRun {
                since: UnixMillis(30),
                revisit: true
            })
        );
    }

    #[test]
    fn test_revisit_from() {
        assert_eq!(revisit_from(None, UnixMillis(100)), UnixMillis(100));
        assert_eq!(
            revisit_from(Some(UnixMillis(50)), UnixMillis(100)),
            UnixMillis(50)
        );
        assert_eq!(
            revisit_from(Some(UnixMillis(150)), UnixMillis(100)),
            UnixMillis(100)
        );
    }

    // Indexing against the mock lila.

    struct Setup {
        _fixture_dir: TempDir,
        _db_dir: TempDir,
        mock: MockLila,
        indexer: Arc<Indexer>,
        players: Vec<UserId>,
        games: Vec<Vec<FixtureGame>>,
    }

    async fn setup_mock(
        players: usize,
        games: impl Fn(usize) -> usize,
        indexers: usize,
        faults: Faults,
        config: IndexerConfig,
    ) -> Setup {
        let fixture_dir = TempDir::new("fixture");
        let mut users: Vec<UserId> = (0..players)
            .map(|i| UserId::from_name(&format!("player{i}")).unwrap())
            .collect();
        users.sort();
        let start = days_from_civil(2026, 6, 1) as u64 * MS_PER_DAY;
        let fixture_games = write_synthetic(&fixture_dir.0, &users, games, start, 42).unwrap();
        let mock = MockLila::start(MockConfig {
            fixture: fixture_dir.0.clone(),
            data_end: days_from_civil(2026, 11, 1) as u64 * MS_PER_DAY,
            max_streams: indexers,
            casual: 0.5,
            noise: Noise::default(),
            faults,
            games_per_second: 20_000.0,
            ..MockConfig::default()
        })
        .await
        .unwrap();
        let db_dir = TempDir::new("indexer-db");
        let db = Arc::new(
            PlayerDb::open(
                &db_dir.0,
                PlayerDbConfig {
                    cache_bytes: 16 << 20,
                    ..PlayerDbConfig::default()
                },
            )
            .unwrap(),
        );
        let lila = Arc::new(LilaClient::new(LilaClientConfig {
            url: mock.url(),
            streams: indexers,
            read_timeout: Duration::from_millis(500),
            ..LilaClientConfig::default()
        }));
        let indexer = Indexer::start(
            db,
            lila,
            IndexerConfig {
                indexers,
                retry_delay: Duration::from_millis(20),
                retries: 20,
                ..config
            },
        );
        Setup {
            _fixture_dir: fixture_dir,
            _db_dir: db_dir,
            mock,
            indexer,
            players: users,
            games: fixture_games,
        }
    }

    impl Setup {
        /// Requests all players and waits until indexing is complete.
        async fn index_all(&self) {
            let mut tickets = Vec::new();
            for user in &self.players {
                tickets.push(self.indexer.request(user).await.unwrap());
            }
            for mut ticket in tickets {
                while !ticket.progress().done {
                    ticket.changed().await;
                }
            }
            // Wait for rebuilds.
            for _ in 0..500 {
                if self.indexer.queue().stats().busy == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        fn check(&self) {
            let db = self.indexer.db();
            let reader = db.reader();
            for (user, games) in self.players.iter().zip(&self.games) {
                let status = reader.status(db.user_hash(user)).unwrap().expect("status");
                assert_eq!(status.games as usize, games.len(), "{user}");
                assert!(status.name.eq_ignore_ascii_case(user.as_str()));
                for color in Color::ALL {
                    let expected = games
                        .iter()
                        .filter(|g| !g.sans.is_empty() && user.is_name(&g.players.get(color).name))
                        .count() as u64;
                    let (entry, _) = reader
                        .position(
                            &status,
                            color,
                            &VariantPosition::new(shakmaty::variant::Variant::Chess),
                        )
                        .unwrap();
                    let mut buf = Vec::new();
                    let mut entry = entry;
                    entry.pack_entire(&mut buf);
                    let total: u64 =
                        crate::packed::EntryView::<crate::packed::PlayerGroupHeader>::unpack_entire(
                            &buf,
                        )
                        .cells()
                        .map(|cell| cell.stats.games())
                        .sum();
                    assert_eq!(total, expected, "{user} {color}");
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_index_all() {
        let setup = setup_mock(
            12,
            |p| [0, 1, 7, 300, 2500][p % 5],
            3,
            Faults::default(),
            IndexerConfig {
                chunk_games: 100,
                rebuild_min_games: 200,
                ..IndexerConfig::default()
            },
        )
        .await;
        setup.index_all().await;
        setup.check();
        let m = &setup.indexer.metrics;
        assert_eq!(m.runs_complete.get(), 12);
        assert!(setup.indexer.db().counters.rebuilds.get() >= 3);

        // Within the cooldown, no new runs.
        let ticket = setup.indexer.request(&setup.players[0]).await.unwrap();
        assert!(ticket.progress().done);
        // Only meaningful without aborted streams: from the point of view
        // of the mock, the next request can briefly overlap.
        if m.stream_retries.get() == 0 {
            let stats = setup.mock.stats();
            assert_eq!(stats.violations, 0, "{stats:?}");
            assert!(stats.max_active_streams <= 3, "{stats:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_index_with_faults() {
        let setup = setup_mock(
            8,
            |p| [50, 400, 1200, 3000][p % 4],
            2,
            Faults {
                stall: 0.2,
                stall_duration: Duration::from_millis(1500),
                disconnect: 0.3,
                server_error: 0.1,
                malformed: 0.001,
                ..Faults::default()
            },
            IndexerConfig {
                chunk_games: 50,
                max_run: Some(Duration::from_millis(100)),
                rebuild_min_games: 500,
                policy: RunPolicy {
                    cooldown: Duration::ZERO,
                    ..RunPolicy::default()
                },
                ..IndexerConfig::default()
            },
        )
        .await;
        setup.index_all().await;
        // Malformed lines lose their games, so request again until all are in
        // (the mock picks faults per request).
        for _ in 0..10 {
            let db = setup.indexer.db();
            let missing = setup.players.iter().zip(&setup.games).any(|(user, games)| {
                db.status(db.user_hash(user))
                    .unwrap()
                    .is_none_or(|s| (s.games as usize) < games.len())
            });
            if !missing {
                break;
            }
            // Games lost to malformed lines are behind the checkpoint: reset
            // the players to get them again.
            for (user, games) in setup.players.iter().zip(&setup.games) {
                if db
                    .status(db.user_hash(user))
                    .unwrap()
                    .is_none_or(|s| (s.games as usize) < games.len())
                {
                    let mut status = db.status(db.user_hash(user)).unwrap().unwrap();
                    status.latest_created_at = crate::player::time::UnixMillis::EPOCH;
                    db.put_status(&status).unwrap();
                }
            }
            setup.index_all().await;
        }
        setup.check();
        let m = &setup.indexer.metrics;
        assert!(m.stream_retries.get() > 0);
        assert!(m.runs_requeued.get() > 0);
    }
}
