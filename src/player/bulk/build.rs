//! The base of the prod-like database: per partition of the scattered
//! games, the games of each player up to a cutoff as sorted SST files of
//! all column families (aggregated and placed as by a rebuild), and the
//! games after the cutoff that the aging step indexes, in tail files.
//!
//! Per player, a plan (deterministic from the user hash and the seed):
//!
//! * unindexed: nothing is written (first runs when the log is replayed);
//! * dormant: indexed once, at the start of the aging window minus an
//!   exponentially distributed delay, never since;
//! * active: indexed up to the start of the aging window, then visited
//!   every `interval` (log-uniform between the bounds, random phase) until
//!   the end of the window, each visit indexing the games created since the
//!   previous one;
//! * new: not indexed before the window, the first index run at a random
//!   time in the window (all games so far), then visited like active
//!   players.
//!
//! ```text
//! tail  := entry*   (by visit, user hash, createdAt)
//! entry := u32(len of the rest) u64(visit) UserHash u64(createdAt)
//!          u8(flags) PlayerGame
//! flags := 1 if the game is not indexed, then without PlayerGame
//! ```
//!
//! Times are those of the fixture (before the shift of the mock), except
//! in the records and statuses written to the database.

use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use rocksdb::{Options, SstFileWriter};

use crate::{
    mock_lila::{
        prepare::name_hash,
        synth::{Synth, game_number, unit},
    },
    packed::GameId,
    player::{
        PlayerDb, UserHash,
        aggregate::{Aggregation, Placed, cold_bits_for},
        bulk::{
            USER_HASH_KEY_FILE,
            population::{Member, Origin},
            scatter::{self, Partitions},
        },
        db::{COLD_CF, GAME_IDS_CF, GAMES_CF, STATUS_CF, WARM_CF},
        game::BorrowedPlayerGame,
        keys::{ColdGroupKey, GameIdKey, GameKey, Generation, PlayerGameIndex, StatusKey, WarmKey},
        status::PlayerStatus,
        time::UnixMillis,
    },
    util::civil::MS_PER_DAY,
};

/// Shares of the plans, and the aging window.
#[derive(Debug, Clone)]
pub struct AgingPlan {
    /// Start and end of the aging window (fixture time, ms).
    pub window_start: u64,
    pub window_end: u64,
    /// Requests players: shares that are not indexed at all, dormant, and
    /// new (the rest is active).
    pub requests_unindexed: f64,
    pub requests_dormant: f64,
    pub requests_new: f64,
    /// Sampled players: shares that are new and active (the rest is
    /// dormant).
    pub sample_new: f64,
    pub sample_active: f64,
    /// Mean time from the last index run of a dormant player to the start
    /// of the window (ms, exponentially distributed).
    pub dormant_delay: u64,
    /// Bounds of the visit interval of active players (ms).
    pub min_interval: u64,
    pub max_interval: u64,
    pub seed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Unindexed,
    Dormant {
        cutoff: u64,
    },
    /// Indexed up to `cutoff` (0 for a new player, whose first index run is
    /// the first visit in the window), then visited.
    Active {
        cutoff: u64,
        first_visit: u64,
        interval: u64,
    },
}

impl AgingPlan {
    fn unit(&self, uh: UserHash, salt: u64) -> f64 {
        unit(self.seed, uh.to_u64(), salt)
    }

    /// The plan of a player with games created from `first`.
    pub fn plan(&self, uh: UserHash, origin: Origin, first: u64) -> Plan {
        #[derive(PartialEq)]
        enum Kind {
            Dormant,
            New,
            Active,
        }
        let u = self.unit(uh, 100);
        let kind = match origin {
            Origin::Requests => {
                let dormant = self.requests_unindexed + self.requests_dormant;
                if u < self.requests_unindexed {
                    return Plan::Unindexed;
                } else if u < dormant {
                    Kind::Dormant
                } else if u < dormant + self.requests_new {
                    Kind::New
                } else {
                    Kind::Active
                }
            }
            Origin::Sample if u < self.sample_new => Kind::New,
            Origin::Sample if u < self.sample_new + self.sample_active => Kind::Active,
            Origin::Sample => Kind::Dormant,
        };
        let (lo, hi) = (self.min_interval as f64, self.max_interval as f64);
        let interval = ((lo.ln() + self.unit(uh, 101) * (hi.ln() - lo.ln())).exp() as u64).max(1);
        match kind {
            Kind::New => Plan::Active {
                cutoff: 0,
                first_visit: self.window_start
                    + (self.unit(uh, 104) * (self.window_end - self.window_start) as f64) as u64,
                interval,
            },
            Kind::Dormant if first < self.window_start => {
                let delay = -(1.0 - self.unit(uh, 103)).ln() * self.dormant_delay as f64;
                Plan::Dormant {
                    cutoff: self.window_start.saturating_sub(delay as u64).max(first),
                }
            }
            _ => Plan::Active {
                cutoff: self.window_start,
                first_visit: self.window_start + (self.unit(uh, 102) * interval as f64) as u64,
                interval,
            },
        }
    }
}

/// `createdAt` by the rank of a row among the rows of its day, as for the
/// mock fixture (`mock_lila::prepare`).
pub struct CreatedAt {
    before: HashMap<(u32, u32), u64>,
    totals: HashMap<u32, u64>,
}

impl CreatedAt {
    pub fn new(mut days: Vec<((u32, u32), u32)>) -> CreatedAt {
        days.sort_unstable();
        let mut before = HashMap::with_capacity(days.len());
        let mut totals: HashMap<u32, u64> = HashMap::new();
        for ((day, rg), count) in days {
            let total = totals.entry(day).or_insert(0);
            before.insert((day, rg), *total);
            *total += u64::from(count);
        }
        CreatedAt { before, totals }
    }

    pub fn of(&self, day: u32, row_group: u32, rank: u32) -> u64 {
        let rank = self.before[&(day, row_group)] + u64::from(rank);
        let fraction = (rank as f64 + 0.5) / self.totals[&day] as f64;
        u64::from(day) * MS_PER_DAY + (fraction * MS_PER_DAY as f64) as u64
    }
}

/// Writes sorted keys of a column family to SST files of about a target
/// size.
struct CfWriter<'a> {
    dir: PathBuf,
    partition: usize,
    opts: &'a Options,
    writer: Option<SstFileWriter<'a>>,
    files: usize,
    target: u64,
    bytes: u64,
    dry_run: bool,
}

impl<'a> CfWriter<'a> {
    fn new(
        cf: &'static str,
        dir: &Path,
        partition: usize,
        opts: &'a Options,
        target: u64,
        dry_run: bool,
    ) -> CfWriter<'a> {
        CfWriter {
            dir: dir.join(cf),
            partition,
            opts,
            writer: None,
            files: 0,
            target,
            bytes: 0,
            dry_run,
        }
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), rocksdb::Error> {
        if self.dry_run {
            self.bytes += (key.len() + value.len()) as u64;
            return Ok(());
        }
        if self.writer.is_none() {
            let writer = SstFileWriter::create(self.opts);
            writer.open(
                self.dir
                    .join(format!("p{:05}_{:04}.sst", self.partition, self.files)),
            )?;
            self.files += 1;
            self.writer = Some(writer);
        }
        let writer = self.writer.as_mut().expect("writer");
        writer.put(key, value)?;
        if writer.file_size() >= self.target {
            self.finish_file()?;
        }
        Ok(())
    }

    fn finish_file(&mut self) -> Result<(), rocksdb::Error> {
        if let Some(mut writer) = self.writer.take() {
            writer.finish()?;
            self.bytes += writer.file_size();
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BuildConfig<'a> {
    pub scatter_dir: &'a Path,
    pub out: &'a Path,
    pub partitions: Partitions,
    pub members: &'a [Member],
    pub threads: usize,
    pub synth: Synth,
    pub plan: AgingPlan,
    pub sst_target_bytes: u64,
    /// Only plan and count games (no aggregation; SST sizes are raw bytes
    /// of the records).
    pub dry_run: bool,
}

#[derive(Debug, Default)]
pub struct BuildStats {
    pub players_unindexed: AtomicU64,
    pub players_dormant: AtomicU64,
    pub players_active: AtomicU64,
    pub players_new: AtomicU64,
    pub players_with_base: AtomicU64,
    pub base_games: AtomicU64,
    pub tail_games: AtomicU64,
    pub tail_visits: AtomicU64,
    pub unindexed_games: AtomicU64,
    pub after_window_games: AtomicU64,
    pub duplicates: AtomicU64,
    pub warm_entries: AtomicU64,
    pub cold_groups: AtomicU64,
    pub sst_bytes: [AtomicU64; 5],
    pub tail_bytes: AtomicU64,
}

pub const CFS: [&str; 5] = [STATUS_CF, GAMES_CF, GAME_IDS_CF, WARM_CF, COLD_CF];

impl BuildStats {
    pub fn summary(&self) -> String {
        let l = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let sst: Vec<String> = CFS
            .iter()
            .zip(&self.sst_bytes)
            .map(|(cf, b)| format!("{cf} {:.2} GiB", l(b) as f64 / (1u64 << 30) as f64))
            .collect();
        format!(
            "players: {} unindexed, {} dormant, {} active, {} new, {} with a base; games: {} base, {} tail in {} visits, {} of unindexed players, {} after the cutoff not indexed, {} duplicates; {} warm entries, {} cold groups; {}; tails {:.2} GiB",
            l(&self.players_unindexed),
            l(&self.players_dormant),
            l(&self.players_active),
            l(&self.players_new),
            l(&self.players_with_base),
            l(&self.base_games),
            l(&self.tail_games),
            l(&self.tail_visits),
            l(&self.unindexed_games),
            l(&self.after_window_games),
            l(&self.duplicates),
            l(&self.warm_entries),
            l(&self.cold_groups),
            sst.join(", "),
            l(&self.tail_bytes) as f64 / (1u64 << 30) as f64,
        )
    }
}

/// A game of a player after sorting.
struct Sorted<'a> {
    created_at: u64,
    game: Option<BorrowedPlayerGame<'a>>,
}

/// Builds the base SST files and the tail files of all partitions. SST
/// files of a column family are in `out/<cf>/`, in key order by name.
pub fn build(config: &BuildConfig<'_>) -> io::Result<BuildStats> {
    for cf in CFS {
        fs::create_dir_all(config.out.join(cf))?;
    }
    fs::create_dir_all(config.out.join("tail"))?;
    fs::copy(
        config.scatter_dir.join(USER_HASH_KEY_FILE),
        config.out.join(USER_HASH_KEY_FILE),
    )?;
    let created_at = CreatedAt::new(scatter::read_days(config.scatter_dir)?);
    let names = scatter::read_names(config.scatter_dir)?;
    let origins: HashMap<u64, Origin> = config
        .members
        .iter()
        .map(|m| (m.name_hash, m.origin))
        .collect();
    let origin_of = |uh: &UserHash| -> Origin {
        names
            .get(uh)
            .and_then(|name| origins.get(&name_hash(name.as_bytes())))
            .copied()
            .unwrap_or(Origin::Sample)
    };

    let stats = BuildStats::default();
    let next = AtomicUsize::new(0);
    let started = Instant::now();
    let error: Mutex<Option<io::Error>> = Mutex::new(None);
    let opts: Vec<Options> = CFS.iter().map(|cf| PlayerDb::sst_options(cf)).collect();
    thread::scope(|s| {
        for _ in 0..config.threads {
            s.spawn(|| {
                loop {
                    let p = next.fetch_add(1, Ordering::Relaxed);
                    if p >= config.partitions.count() || error.lock().expect("error").is_some() {
                        break;
                    }
                    let result =
                        build_partition(config, p, &opts, &created_at, &names, &origin_of, &stats);
                    if let Err(err) = result {
                        *error.lock().expect("error") = Some(err);
                        break;
                    }
                    if p.is_multiple_of(32) {
                        log::info!(
                            "partition {p}/{} after {:.0} s: {}",
                            config.partitions.count(),
                            started.elapsed().as_secs_f64(),
                            stats.summary()
                        );
                    }
                }
            });
        }
    });
    if let Some(err) = error.into_inner().expect("error") {
        return Err(err);
    }
    log::info!(
        "built in {:.0} s: {}",
        started.elapsed().as_secs_f64(),
        stats.summary()
    );
    Ok(stats)
}

fn rocks(err: rocksdb::Error) -> io::Error {
    io::Error::other(err)
}

#[allow(clippy::too_many_arguments)]
fn build_partition(
    config: &BuildConfig<'_>,
    p: usize,
    opts: &[Options],
    created_at: &CreatedAt,
    names: &HashMap<UserHash, String>,
    origin_of: &(impl Fn(&UserHash) -> Origin + Sync),
    stats: &BuildStats,
) -> io::Result<()> {
    let bytes = fs::read(Partitions::path(config.scatter_dir, p))?;
    let mut entries: Vec<(UserHash, u64, u64, scatter::Entry<'_>)> = scatter::entries(&bytes)
        .map(|b| {
            let e = scatter::Entry::unpack(b);
            (e.uh, created_at.of(e.day, e.row_group, e.rank), e.row, e)
        })
        .collect();
    entries.sort_unstable_by_key(|(uh, created, row, _)| (*uh, *created, *row));

    let mut writers: Vec<CfWriter<'_>> = CFS
        .iter()
        .zip(opts)
        .map(|(cf, opts)| {
            CfWriter::new(
                cf,
                config.out,
                p,
                opts,
                config.sst_target_bytes,
                config.dry_run,
            )
        })
        .collect();
    let mut tail: Vec<(u64, UserHash, u64, Option<Vec<u8>>)> = Vec::new();
    let synth = &config.synth;

    let mut start = 0;
    let mut buf = Vec::new();
    while start < entries.len() {
        let uh = entries[start].0;
        let end = start + entries[start..].partition_point(|e| e.0 == uh);
        // Strictly increasing per player, as in the fixture.
        let mut games: Vec<Sorted<'_>> = Vec::with_capacity(end - start);
        let mut previous: Option<u64> = None;
        for (_, created, _, e) in &entries[start..end] {
            let created = match previous {
                Some(previous) if *created <= previous => previous + 1,
                _ => *created,
            };
            previous = Some(created);
            let game = e.game.map(|mut g| {
                g.month = crate::packed::Month::from_unix_ms(
                    synth.shifted(created)
                        + synth.duration(game_number(g.id), u64::from(e.plies), g.speed),
                );
                g
            });
            games.push(Sorted {
                created_at: created,
                game,
            });
        }
        start = end;

        let plan = config.plan.plan(uh, origin_of(&uh), games[0].created_at);
        let (cutoff, visits) = match plan {
            Plan::Unindexed => {
                stats.players_unindexed.fetch_add(1, Ordering::Relaxed);
                stats
                    .unindexed_games
                    .fetch_add(games.len() as u64, Ordering::Relaxed);
                continue;
            }
            Plan::Dormant { cutoff } => {
                stats.players_dormant.fetch_add(1, Ordering::Relaxed);
                (cutoff, None)
            }
            Plan::Active {
                cutoff,
                first_visit,
                interval,
            } => {
                if cutoff == 0 {
                    stats.players_new.fetch_add(1, Ordering::Relaxed);
                } else {
                    stats.players_active.fetch_add(1, Ordering::Relaxed);
                }
                (cutoff, Some((first_visit, interval)))
            }
        };

        // Base: the games up to the cutoff.
        let n_base = games.partition_point(|g| g.created_at <= cutoff);
        if n_base > 0 {
            let mut status = PlayerStatus::new(
                names
                    .get(&uh)
                    .and_then(|name| crate::player::UserId::from_name(name).ok())
                    .expect("name of player"),
            );
            if let Some(name) = names.get(&uh) {
                status.name = name.clone();
            }
            let mut aggregation = Aggregation::new();
            let mut ids: Vec<(GameId, PlayerGameIndex)> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for g in &games[..n_base] {
                status.latest_created_at = UnixMillis(synth.shifted(g.created_at));
                let Some(game) = &g.game else {
                    continue;
                };
                if !seen.insert(game.id) {
                    stats.duplicates.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let seq = status.next_index();
                status.games += 1;
                buf.clear();
                game.pack(&mut buf);
                writers[1]
                    .put(GameKey::new(uh, seq).as_ref(), &buf)
                    .map_err(rocks)?;
                ids.push((game.id, seq));
                if !config.dry_run {
                    aggregation.add_game(seq, game);
                }
            }
            stats
                .base_games
                .fetch_add(u64::from(status.games), Ordering::Relaxed);
            let mut id_keys: Vec<(GameIdKey, PlayerGameIndex)> = ids
                .into_iter()
                .map(|(id, seq)| (GameIdKey::new(uh, id), seq))
                .collect();
            id_keys.sort_unstable_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
            for (key, seq) in &id_keys {
                writers[2]
                    .put(key.as_ref(), &seq.to_bytes())
                    .map_err(rocks)?;
            }
            let placed = aggregation.place(|_, items| cold_bits_for(items));
            let generation = Generation(0);
            status.cold_bits = placed.cold_bits;
            status.games_at_rebuild = status.games;
            status.indexed_at = UnixMillis(synth.shifted(cutoff));
            let Placed { warm, cold, .. } = placed;
            stats
                .warm_entries
                .fetch_add(warm.len() as u64, Ordering::Relaxed);
            stats
                .cold_groups
                .fetch_add(cold.len() as u64, Ordering::Relaxed);
            for (color, key, mut entry) in warm {
                buf.clear();
                entry.pack_entire(&mut buf);
                writers[3]
                    .put(WarmKey::new(uh, generation, color, &key).as_ref(), &buf)
                    .map_err(rocks)?;
            }
            for (color, bucket, mut group) in cold {
                buf.clear();
                group.pack_entire(&mut buf);
                writers[4]
                    .put(
                        ColdGroupKey::new(uh, generation, color, bucket).as_ref(),
                        &buf,
                    )
                    .map_err(rocks)?;
            }
            buf.clear();
            status.pack(&mut buf);
            writers[0]
                .put(StatusKey::new(uh).as_ref(), &buf)
                .map_err(rocks)?;
            stats.players_with_base.fetch_add(1, Ordering::Relaxed);
        }

        // Tail: the games of the visits in the window.
        let mut after = 0;
        match visits {
            Some((first_visit, interval)) if first_visit <= config.plan.window_end => {
                let last_visit =
                    first_visit + (config.plan.window_end - first_visit) / interval * interval;
                let mut visit_count = 0;
                let mut last = None;
                for g in &games[n_base..] {
                    if g.created_at > last_visit {
                        after += 1;
                        continue;
                    }
                    let visit = if g.created_at <= first_visit {
                        first_visit
                    } else {
                        first_visit + (g.created_at - first_visit).div_ceil(interval) * interval
                    };
                    if last != Some(visit) {
                        visit_count += 1;
                        last = Some(visit);
                    }
                    let record = g.game.as_ref().map(|game| {
                        let mut b = Vec::new();
                        game.pack(&mut b);
                        b
                    });
                    tail.push((visit, uh, g.created_at, record));
                    stats.tail_games.fetch_add(1, Ordering::Relaxed);
                }
                stats.tail_visits.fetch_add(visit_count, Ordering::Relaxed);
            }
            _ => after = games.len() - n_base,
        }
        stats
            .after_window_games
            .fetch_add(after as u64, Ordering::Relaxed);
    }
    for (w, bytes) in writers.iter_mut().zip(&stats.sst_bytes) {
        w.finish_file().map_err(rocks)?;
        bytes.fetch_add(w.bytes, Ordering::Relaxed);
    }

    if config.dry_run {
        let bytes: usize = tail
            .iter()
            .map(|t| 29 + t.3.as_ref().map_or(0, Vec::len))
            .sum();
        stats.tail_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        return Ok(());
    }
    tail.sort_unstable_by_key(|(visit, uh, created, _)| (*visit, *uh, *created));
    let path = tail_path(config.out, p);
    let mut w = BufWriter::new(File::create(&path)?);
    let mut written = 0u64;
    for (visit, uh, created, record) in &tail {
        let len = 8 + 8 + 8 + 1 + record.as_ref().map_or(0, Vec::len);
        w.write_all(&(len as u32).to_le_bytes())?;
        w.write_all(&visit.to_le_bytes())?;
        w.write_all(uh.as_bytes())?;
        w.write_all(&created.to_le_bytes())?;
        w.write_all(&[u8::from(record.is_none())])?;
        if let Some(record) = record {
            w.write_all(record)?;
        }
        written += 4 + len as u64;
    }
    w.flush()?;
    stats.tail_bytes.fetch_add(written, Ordering::Relaxed);
    Ok(())
}

pub fn tail_path(out: &Path, p: usize) -> PathBuf {
    out.join("tail").join(format!("p{p:05}.bin"))
}

/// The SST files of a column family, in key order.
pub fn sst_files(out: &Path, cf: &str) -> io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(out.join(cf))?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    files.retain(|f| f.extension().is_some_and(|e| e == "sst"));
    files.sort();
    Ok(files)
}
