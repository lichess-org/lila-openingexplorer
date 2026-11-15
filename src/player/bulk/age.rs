//! Aging of the bulk-built base: the visits of the tail files through the
//! live write path (`write_chunk` in chunks, rebuild triggers with batch
//! and ingest rebuilds, `DeleteRange` of old generations), in time order
//! across players, so that memtables, L0 and the upper levels hold mixed
//! players, merge operands and drift.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    fs::File,
    io::{self, BufReader, Read},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use crossbeam::channel;

use crate::{
    mock_lila::synth::Synth,
    player::{
        PlayerDb, UserHash, UserId,
        bulk::{build::tail_path, scatter::Partitions},
        game::{BorrowedPlayerGame, PlayerGame},
        status::PlayerStatus,
        time::UnixMillis,
    },
};

/// An entry of a tail file.
struct TailEntry {
    visit: u64,
    uh: UserHash,
    created_at: u64,
    record: Option<Vec<u8>>,
}

struct TailReader {
    reader: BufReader<File>,
}

impl TailReader {
    fn next(&mut self) -> io::Result<Option<TailEntry>> {
        let mut len = [0; 4];
        match self.reader.read_exact(&mut len) {
            Ok(()) => (),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err),
        }
        let mut b = vec![0; u32::from_le_bytes(len) as usize];
        self.reader.read_exact(&mut b)?;
        Ok(Some(TailEntry {
            visit: u64::from_le_bytes(b[..8].try_into().expect("8 bytes")),
            uh: UserHash::from_bytes(b[8..16].try_into().expect("8 bytes")),
            created_at: u64::from_le_bytes(b[16..24].try_into().expect("8 bytes")),
            record: (b[24] == 0).then(|| b[25..].to_vec()),
        }))
    }
}

/// `createdAt` (fixture time) of a game of a visit, and its record if it
/// is indexed.
type VisitGame = (u64, Option<Vec<u8>>);

/// The games of a player indexed by one visit.
struct Visit {
    uh: UserHash,
    visit: u64,
    games: Vec<VisitGame>,
}

#[derive(Debug, Clone)]
pub struct AgeConfig<'a> {
    pub out: &'a Path,
    pub partitions: Partitions,
    pub workers: usize,
    pub synth: Synth,
    pub chunk_games: usize,
    pub rebuild_min_games: u32,
    pub rebuild_factor: f64,
    /// Stop after visits up to this time (fixture time).
    pub until: Option<u64>,
}

#[derive(Debug, Default)]
pub struct AgeStats {
    pub visits: AtomicU64,
    pub first_runs: AtomicU64,
    pub games: AtomicU64,
    pub known: AtomicU64,
    pub chunks: AtomicU64,
    pub rebuilds: AtomicU64,
    pub rebuilds_ingested: AtomicU64,
    pub rebuild_micros: AtomicU64,
    /// Visit time of the latest dispatched visit.
    pub clock: AtomicU64,
}

/// Runs the visits of the tail files through the database.
pub fn age(
    db: &PlayerDb,
    names: &HashMap<UserHash, String>,
    config: &AgeConfig<'_>,
) -> io::Result<AgeStats> {
    let mut readers: Vec<TailReader> = (0..config.partitions.count())
        .map(|p| {
            Ok(TailReader {
                reader: BufReader::with_capacity(1 << 20, File::open(tail_path(config.out, p))?),
            })
        })
        .collect::<io::Result<_>>()?;
    let stats = AgeStats::default();
    let started = Instant::now();
    let failed = AtomicU64::new(0);

    thread::scope(|s| -> io::Result<()> {
        let senders: Vec<channel::Sender<Visit>> = (0..config.workers)
            .map(|_| {
                let (tx, rx) = channel::bounded::<Visit>(64);
                let (stats, failed) = (&stats, &failed);
                s.spawn(move || {
                    for visit in rx {
                        if let Err(err) = run_visit(db, names, config, stats, visit) {
                            log::error!("visit: {err}");
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
                tx
            })
            .collect();

        // Merge the tails by (visit, user hash, createdAt).
        let mut heap = BinaryHeap::new();
        let mut heads: Vec<Option<TailEntry>> = Vec::with_capacity(readers.len());
        for (i, r) in readers.iter_mut().enumerate() {
            let head = r.next()?;
            if let Some(e) = &head {
                heap.push(Reverse((e.visit, e.uh, e.created_at, i)));
            }
            heads.push(head);
        }
        let mut current: Option<Visit> = None;
        let mut last_log = Instant::now();
        let dispatch = |visit: Visit| {
            stats.clock.store(visit.visit, Ordering::Relaxed);
            let worker = (visit.uh.to_u64() % config.workers as u64) as usize;
            senders[worker].send(visit).expect("worker");
        };
        while let Some(Reverse((visit, uh, _, i))) = heap.pop() {
            let entry = heads[i].take().expect("head");
            if let Some(next) = readers[i].next()? {
                heap.push(Reverse((next.visit, next.uh, next.created_at, i)));
                heads[i] = Some(next);
            }
            if config.until.is_some_and(|until| visit > until) {
                break;
            }
            match &mut current {
                Some(c) if c.uh == uh && c.visit == visit => {
                    c.games.push((entry.created_at, entry.record));
                }
                _ => {
                    if let Some(done) = current.take() {
                        dispatch(done);
                    }
                    current = Some(Visit {
                        uh,
                        visit,
                        games: vec![(entry.created_at, entry.record)],
                    });
                }
            }
            if last_log.elapsed() >= Duration::from_secs(30) {
                last_log = Instant::now();
                log_progress(db, &stats, started);
            }
        }
        if let Some(done) = current.take() {
            dispatch(done);
        }
        drop(senders);
        Ok(())
    })?;
    log_progress(db, &stats, started);
    let failed = failed.load(Ordering::Relaxed);
    if failed > 0 {
        return Err(io::Error::other(format!("{failed} visits failed")));
    }
    Ok(stats)
}

fn log_progress(db: &PlayerDb, stats: &AgeStats, started: Instant) {
    let l = |c: &AtomicU64| c.load(Ordering::Relaxed);
    let elapsed = started.elapsed().as_secs_f64();
    let clock = l(&stats.clock);
    let pending = db
        .property("rocksdb.estimate-pending-compaction-bytes")
        .ok()
        .flatten()
        .unwrap_or_default();
    let l0: Vec<String> = PlayerDb::column_families()
        .iter()
        .map(|cf| {
            db.cf_int_property(cf, "rocksdb.num-files-at-level0")
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    log::info!(
        "aging {elapsed:.0} s, at {} (fixture time): {} visits ({} first runs), {} games ({:.0}/s), {} known, {} chunks, {} rebuilds ({} ingested, {:.0} s), L0 files {}, pending compaction {}",
        crate::packed::Month::from_unix_ms(clock),
        l(&stats.visits),
        l(&stats.first_runs),
        l(&stats.games),
        l(&stats.games) as f64 / elapsed,
        l(&stats.known),
        l(&stats.chunks),
        l(&stats.rebuilds),
        l(&stats.rebuilds_ingested),
        l(&stats.rebuild_micros) as f64 / 1e6,
        l0.join("/"),
        pending,
    );
}

fn run_visit(
    db: &PlayerDb,
    names: &HashMap<UserHash, String>,
    config: &AgeConfig<'_>,
    stats: &AgeStats,
    visit: Visit,
) -> Result<(), crate::player::PlayerDbError> {
    let synth = &config.synth;
    let name = names.get(&visit.uh).expect("name of player");
    let mut status = match db.status(visit.uh)? {
        Some(status) => status,
        None => {
            stats.first_runs.fetch_add(1, Ordering::Relaxed);
            PlayerStatus::new(UserId::from_name(name).expect("valid name"))
        }
    };
    status.name = name.clone();
    let chunks: Vec<&[VisitGame]> = visit.games.chunks(config.chunk_games).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let games: Vec<PlayerGame> = chunk
            .iter()
            .filter_map(|(_, record)| {
                record
                    .as_ref()
                    .map(|r| BorrowedPlayerGame::unpack(r).into_owned())
            })
            .collect();
        let latest = chunk.iter().map(|(c, _)| *c).max().expect("chunk");
        status.latest_created_at = status
            .latest_created_at
            .max(UnixMillis(synth.shifted(latest)));
        if i + 1 == chunks.len() {
            status.indexed_at = UnixMillis(synth.shifted(visit.visit));
        }
        let chunk_stats = db.write_chunk(&mut status, &games)?;
        stats.games.fetch_add(chunk_stats.games, Ordering::Relaxed);
        stats.known.fetch_add(chunk_stats.known, Ordering::Relaxed);
        stats.chunks.fetch_add(1, Ordering::Relaxed);
    }
    stats.visits.fetch_add(1, Ordering::Relaxed);
    if db.rebuild_due(&status, config.rebuild_min_games, config.rebuild_factor) {
        let started = Instant::now();
        if let Some(rebuild) = db.rebuild(visit.uh)? {
            stats.rebuilds.fetch_add(1, Ordering::Relaxed);
            stats
                .rebuilds_ingested
                .fetch_add(u64::from(rebuild.ingested), Ordering::Relaxed);
        }
        stats
            .rebuild_micros
            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
    Ok(())
}
