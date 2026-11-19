use std::{
    collections::HashSet,
    fs, io, mem,
    path::{Path, PathBuf},
    time::Instant,
};

use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamily, ColumnFamilyDescriptor, DB, DBCompressionType, DBPath,
    Direction, IngestExternalFileOptions, IteratorMode, LiveFile, MergeOperands, Options,
    ReadOptions, SnapshotWithThreadMode, SstFileWriter, WriteBatch, WriteOptions,
    statistics::{StatsLevel, Ticker},
};
use shakmaty::{Color, variant::VariantPosition};
use thiserror::Error;

use crate::{
    packed::{
        CellKey, ColdGroup, ColdGroupView, GameId, PlayerEntry, PlayerGroupHeader, PositionKey,
    },
    player::{
        MAX_PLY, UserHash, UserHasher, UserId,
        aggregate::{Aggregation, Placed, cold_bits_for, find_visit},
        game::{BorrowedPlayerGame, PlayerGame},
        keys::{
            ColdBucket, ColdGroupKey, GameIdKey, GameKey, Generation, PlayerGameIndex, StatusKey,
            WarmKey,
        },
        status::PlayerStatus,
    },
    sst::SstConfig,
    util::{Counter, Exposition, micros_to_seconds},
};

pub const STATUS_CF: &str = "player_status";
pub const GAMES_CF: &str = "player_games";
pub const GAME_IDS_CF: &str = "player_game_ids";
pub const WARM_CF: &str = "player_warm";
pub const COLD_CF: &str = "player_cold";
pub const META_CF: &str = "player_meta";

const PLAYER_CFS: [&str; 5] = [STATUS_CF, GAMES_CF, GAME_IDS_CF, WARM_CF, COLD_CF];

const ALL_CFS: [&str; 6] = {
    let [status, games, game_ids, warm, cold] = PLAYER_CFS;
    [status, games, game_ids, warm, cold, META_CF]
};

/// Key of the `UserHasher` key in the meta column family.
const USER_HASH_KEY: &[u8] = b"user_hash_key";

/// Subdirectory of the database for SST files of rebuilds, before they are
/// ingested.
const REBUILD_DIR: &str = "rebuild";

#[derive(Debug, Error)]
pub enum PlayerDbError {
    #[error("rocksdb: {0}")]
    RocksDb(#[from] rocksdb::Error),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("invalid user hash key")]
    InvalidUserHashKey,
    #[error("user hash key differs from the key of the database")]
    UserHashKeyMismatch,
    #[error("database has players, but no user hash key")]
    MissingUserHashKey,
}

#[derive(Debug, Clone)]
pub struct PlayerDbConfig {
    /// Size of the block cache.
    pub cache_bytes: u64,
    /// Bytes per second of flushes and compactions (`None` for unlimited).
    pub rate_limit: Option<u64>,
    /// Directory of the write ahead log, if not the database directory.
    pub wal_dir: Option<PathBuf>,
    /// Directories for SST files with target sizes: newer data (upper
    /// levels) in the first, older data in later ones (like the NVMe for
    /// upper levels and the HDDs for the last level). Empty for the
    /// database directory.
    pub db_paths: Vec<(PathBuf, u64)>,
    /// Collect RocksDB statistics.
    pub statistics: bool,
    /// Rebuilds with more output are written as SST files and ingested.
    pub ingest_bytes: u64,
    /// Bytes per write batch of a rebuild.
    pub batch_bytes: usize,
    /// Background threads for flushes and compactions.
    pub background_jobs: i32,
    /// With several `db_paths`: the last level holds this many times its
    /// upper level (see `tune_levels`).
    pub level_ratio: f64,
    /// Key of the user hashes for a new database (like the one that SST
    /// files to ingest were built with), or to check against the key of an
    /// existing database. `None` generates one for a new database.
    pub user_hasher: Option<UserHasher>,
}

impl Default for PlayerDbConfig {
    fn default() -> PlayerDbConfig {
        PlayerDbConfig {
            cache_bytes: 1 << 30,
            rate_limit: None,
            wal_dir: None,
            db_paths: Vec::new(),
            statistics: false,
            ingest_bytes: 64 << 20,
            batch_bytes: 4 << 20,
            background_jobs: 4,
            level_ratio: 10.0,
            user_hasher: None,
        }
    }
}

fn warm_merge(_key: &[u8], existing: Option<&[u8]>, operands: &MergeOperands) -> Option<Vec<u8>> {
    let mut entry = PlayerEntry::new();
    if let Some(existing) = existing {
        entry.merge_unpack_entire(existing);
    }
    for operand in operands {
        entry.merge_unpack_entire(operand);
    }
    let mut buf = Vec::new();
    entry.pack_entire(&mut buf);
    Some(buf)
}

fn cold_merge(_key: &[u8], existing: Option<&[u8]>, operands: &MergeOperands) -> Option<Vec<u8>> {
    let mut group = ColdGroup::new();
    for value in existing.into_iter().chain(operands) {
        group.merge_unpack_entire(value);
    }
    let mut buf = Vec::new();
    group.pack_entire(&mut buf);
    Some(buf)
}

fn table_options(cache: &Cache, filter: bool) -> BlockBasedOptions {
    // Compression is set per level by the column family options.
    let mut table_opts = SstConfig {
        block_size: 16 * 1024,
        zstd_level: 0,
        filter,
        separate_key_value_in_data_block: false,
    }
    .table_options();
    table_opts.set_block_cache(cache);
    table_opts.set_cache_index_and_filter_blocks(true);
    table_opts.set_cache_index_and_filter_blocks_with_high_priority(true);
    table_opts.set_pin_top_level_index_and_filter(true);
    table_opts
}

fn cf_options(name: &str, cache: &Cache) -> Options {
    let mut opts = Options::default();
    opts.set_block_based_table_factory(&table_options(cache, name != GAMES_CF));
    opts.set_level_compaction_dynamic_level_bytes(true);
    opts.set_compression_type(DBCompressionType::Lz4);
    opts.set_bottommost_compression_type(DBCompressionType::Zstd);
    opts.set_bottommost_compression_options(-14, 3, 0, 0, true);
    opts.set_write_buffer_size(64 << 20);
    opts.set_target_file_size_base(64 << 20);
    match name {
        WARM_CF => opts.set_merge_operator_associative("player_warm", warm_merge),
        COLD_CF => opts.set_merge_operator_associative("player_cold", cold_merge),
        _ => (),
    }
    opts
}

#[derive(Default)]
pub struct DbCounters {
    pub chunks: Counter,
    pub games_written: Counter,
    pub games_known: Counter,
    pub warm_operands: Counter,
    pub cold_operands: Counter,
    pub cold_candidates: Counter,
    pub cold_false: Counter,
    pub rebuilds: Counter,
    pub rebuilds_ingested: Counter,
    pub rebuild_micros: Counter,
    pub rebuild_bytes: Counter,
}

impl DbCounters {
    pub fn write(&self, out: &mut Exposition) {
        out.counter(
            "chunks_written_total",
            "Chunks of games written.",
            self.chunks.get(),
        );
        out.counter(
            "games_indexed_total",
            "Games written to the index.",
            self.games_written.get(),
        );
        out.counter(
            "games_known_total",
            "Games skipped because they were already indexed.",
            self.games_known.get(),
        );
        out.labeled_counter(
            "merge_operands_total",
            "Merge operands written, by column family.",
            "cf",
            [
                ("warm", self.warm_operands.get()),
                ("cold", self.cold_operands.get()),
            ],
        );
        out.counter(
            "cold_false_total",
            "Cold candidates that replay rejected.",
            self.cold_false.get(),
        );
        out.counter(
            "rebuilds_total",
            "Rebuilds of player indexes.",
            self.rebuilds.get(),
        );
        out.counter(
            "rebuilds_ingested_total",
            "Rebuilds ingested as SST files.",
            self.rebuilds_ingested.get(),
        );
        out.counter(
            "rebuild_seconds_total",
            "Time spent rebuilding.",
            micros_to_seconds(self.rebuild_micros.get()),
        );
        out.counter(
            "rebuild_bytes_total",
            "Bytes written by rebuilds.",
            self.rebuild_bytes.get(),
        );
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ChunkStats {
    pub games: u64,
    /// Games skipped because they were already indexed.
    pub known: u64,
    pub warm_operands: u64,
    pub cold_operands: u64,
    pub batch_bytes: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RebuildStats {
    pub games: u64,
    pub warm_entries: u64,
    pub cold_groups: u64,
    pub bytes: u64,
    pub ingested: bool,
}

fn open_user_hasher(db: &DB, given: Option<&UserHasher>) -> Result<UserHasher, PlayerDbError> {
    let meta = db.cf_handle(META_CF).expect("meta column family");
    if let Some(stored) = db.get_pinned_cf(meta, USER_HASH_KEY)? {
        let stored =
            UserHasher::from_key_bytes(&stored).ok_or(PlayerDbError::InvalidUserHashKey)?;
        if given.is_some_and(|given| *given != stored) {
            return Err(PlayerDbError::UserHashKeyMismatch);
        }
        return Ok(stored);
    }
    let status = db.cf_handle(STATUS_CF).expect("status column family");
    if db.iterator_cf(status, IteratorMode::Start).next().is_some() {
        return Err(PlayerDbError::MissingUserHashKey);
    }
    let hasher = given.cloned().unwrap_or_else(UserHasher::generate);
    let mut write_opts = WriteOptions::default();
    write_opts.set_sync(true);
    db.put_cf_opt(meta, USER_HASH_KEY, hasher.key(), &write_opts)?;
    Ok(hasher)
}

pub struct PlayerDb {
    db: DB,
    path: PathBuf,
    cache: Cache,
    opts: Options,
    warm_opts: Options,
    cold_opts: Options,
    config: PlayerDbConfig,
    hasher: UserHasher,
    pub counters: DbCounters,
}

impl PlayerDb {
    pub fn open(path: &Path, config: PlayerDbConfig) -> Result<PlayerDb, PlayerDbError> {
        let started = Instant::now();
        let mut lru = rocksdb::LruCacheOptions::default();
        lru.set_capacity(config.cache_bytes as usize);
        lru.set_high_pri_pool_ratio(0.5);
        let cache = Cache::new_lru_cache_opts(&lru);

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_max_background_jobs(config.background_jobs);
        opts.set_bytes_per_sync(1 << 20);
        opts.set_db_write_buffer_size(512 << 20);
        opts.set_max_total_wal_size(1 << 30);
        if let Some(rate) = config.rate_limit {
            opts.set_ratelimiter(rate as i64, 100_000, 10);
        }
        if let Some(wal_dir) = &config.wal_dir {
            opts.set_wal_dir(wal_dir);
        }
        if !config.db_paths.is_empty() {
            opts.set_db_paths(&db_paths(&config.db_paths, 1.0)?);
        }
        if config.statistics {
            opts.enable_statistics();
            opts.set_statistics_level(StatsLevel::ExceptDetailedTimers);
        }
        let descriptors = ALL_CFS
            .into_iter()
            .map(|name| {
                let mut cf_opts = cf_options(name, &cache);
                if !config.db_paths.is_empty() {
                    cf_opts.set_cf_paths(&db_paths(&config.db_paths, path_share(name))?);
                }
                Ok(ColumnFamilyDescriptor::new(name, cf_opts))
            })
            .collect::<Result<Vec<_>, rocksdb::Error>>()?;
        let db = DB::open_cf_descriptors(&opts, path, descriptors)?;
        let hasher = open_user_hasher(&db, config.user_hasher.as_ref())?;
        let rebuild_dir = path.join(REBUILD_DIR);
        if rebuild_dir.exists() {
            fs::remove_dir_all(&rebuild_dir)?;
        }
        log::info!("opened {} in {:.1?}", path.display(), started.elapsed());
        let db = PlayerDb {
            db,
            path: path.to_owned(),
            warm_opts: cf_options(WARM_CF, &cache),
            cold_opts: cf_options(COLD_CF, &cache),
            cache,
            opts,
            config,
            hasher,
            counters: DbCounters::default(),
        };
        db.tune_levels()?;
        Ok(db)
    }

    pub fn config(&self) -> &PlayerDbConfig {
        &self.config
    }

    pub fn user_hasher(&self) -> &UserHasher {
        &self.hasher
    }

    pub fn user_hash(&self, user: &UserId) -> UserHash {
        self.hasher.hash(user)
    }

    fn cf(&self, name: &str) -> &ColumnFamily {
        self.db.cf_handle(name).expect("column family")
    }

    pub fn status(&self, uh: UserHash) -> Result<Option<PlayerStatus>, PlayerDbError> {
        Ok(self
            .db
            .get_pinned_cf(self.cf(STATUS_CF), StatusKey::new(uh))?
            .map(|value| PlayerStatus::unpack(&value)))
    }

    pub fn put_status(&self, status: &PlayerStatus) -> Result<(), PlayerDbError> {
        let mut buf = Vec::new();
        status.pack(&mut buf);
        self.db.put_cf(
            self.cf(STATUS_CF),
            StatusKey::new(self.user_hash(&status.user)),
            buf,
        )?;
        Ok(())
    }

    /// Writes a chunk of new games of a player and the status checkpoint
    /// in one atomic batch. Assigns the games their numbers, skipping games
    /// that are already indexed, and updates the status. Reads only the
    /// game ids, never the warm or cold index.
    pub fn write_chunk(
        &self,
        status: &mut PlayerStatus,
        games: &[PlayerGame],
    ) -> Result<ChunkStats, PlayerDbError> {
        let uh = self.user_hash(&status.user);
        let id_keys: Vec<GameIdKey> = games
            .iter()
            .map(|game| GameIdKey::new(uh, game.id))
            .collect();
        let known: Vec<bool> = self
            .db
            .batched_multi_get_cf(self.cf(GAME_IDS_CF), &id_keys, false)
            .into_iter()
            .map(|result| result.map(|value| value.is_some()))
            .collect::<Result<_, _>>()?;
        let mut seen: HashSet<GameId> = HashSet::new();
        let fresh = status.games == 0;

        let mut batch = WriteBatch::default();
        let mut stats = ChunkStats::default();
        let mut aggregation = Aggregation::new();
        let mut buf = Vec::new();
        for (game, known) in games.iter().zip(known) {
            if known || !seen.insert(game.id) {
                stats.known += 1;
                continue;
            }
            let index = status.next_index();
            status.games += 1;
            buf.clear();
            game.pack(&mut buf);
            batch.put_cf(self.cf(GAMES_CF), GameKey::new(uh, index), &buf);
            batch.put_cf(
                self.cf(GAME_IDS_CF),
                GameIdKey::new(uh, game.id),
                index.to_bytes(),
            );
            aggregation.add_game(index, &game.as_borrowed());
            stats.games += 1;
        }

        let current_bits = status.cold_bits;
        let placed = aggregation.place(|counts| {
            if fresh {
                counts.map(cold_bits_for)
            } else {
                current_bits
            }
        });
        if fresh {
            status.cold_bits = placed.cold_bits;
        }
        let generation = status.generation;
        let Placed { warm, cold, .. } = placed;
        for (color, key, mut entry) in warm {
            buf.clear();
            entry.pack_entire(&mut buf);
            batch.merge_cf(
                self.cf(WARM_CF),
                WarmKey::new(uh, generation, color, &key),
                &buf,
            );
            stats.warm_operands += 1;
        }
        for (color, bucket, mut group) in cold {
            buf.clear();
            group.pack_entire(&mut buf);
            batch.merge_cf(
                self.cf(COLD_CF),
                ColdGroupKey::new(uh, generation, color, bucket),
                &buf,
            );
            stats.cold_operands += 1;
        }
        buf.clear();
        status.pack(&mut buf);
        batch.put_cf(self.cf(STATUS_CF), StatusKey::new(uh), &buf);
        stats.batch_bytes = batch.size_in_bytes() as u64;
        self.db.write(batch)?;

        let c = &self.counters;
        c.chunks.inc();
        c.games_written.add(stats.games);
        c.games_known.add(stats.known);
        c.warm_operands.add(stats.warm_operands);
        c.cold_operands.add(stats.cold_operands);
        Ok(stats)
    }

    /// Whether a rebuild of the player is due: the games since the last
    /// rebuild reached `max(min_games, factor * games at the last rebuild)`.
    pub fn rebuild_due(&self, status: &PlayerStatus, min_games: u32, factor: f64) -> bool {
        let since = status.games - status.games_at_rebuild.min(status.games);
        status.games > 0
            && since >= min_games.max((factor * f64::from(status.games_at_rebuild)) as u32)
    }

    /// Rebuilds the warm and cold index of a player from its games, into
    /// the next generation, with the optimal placement for its current
    /// size. Must not run concurrently with writes of the same player.
    pub fn rebuild(&self, uh: UserHash) -> Result<Option<RebuildStats>, PlayerDbError> {
        let started = Instant::now();
        let Some(mut status) = self.status(uh)? else {
            return Ok(None);
        };
        let new = status.generation.next();

        // Leftovers of an interrupted rebuild.
        let (start, end) = uh.generations_range(new, None);
        for cf in [WARM_CF, COLD_CF] {
            self.db.delete_range_cf(self.cf(cf), &start, &end)?;
        }

        let mut aggregation = Aggregation::new();
        let mut stats = RebuildStats::default();
        let (start, end) = uh.range();
        let mut read_opts = ReadOptions::default();
        read_opts.set_iterate_lower_bound(start.clone());
        read_opts.set_iterate_upper_bound(end);
        read_opts.fill_cache(false);
        for item in self.db.iterator_cf_opt(
            self.cf(GAMES_CF),
            read_opts,
            IteratorMode::From(&start, Direction::Forward),
        ) {
            let (key, value) = item?;
            aggregation.add_game(GameKey::index(&key), &BorrowedPlayerGame::unpack(&value));
            stats.games += 1;
        }
        let placed = aggregation.place(|counts| counts.map(cold_bits_for));
        let cold_bits = placed.cold_bits;

        // Pack everything, in key order.
        let mut families = [
            (
                WARM_CF,
                &self.warm_opts,
                Vec::with_capacity(placed.warm.len()),
            ),
            (
                COLD_CF,
                &self.cold_opts,
                Vec::with_capacity(placed.cold.len()),
            ),
        ];
        for (color, key, mut entry) in placed.warm {
            let mut buf = Vec::new();
            entry.pack_entire(&mut buf);
            let key = WarmKey::new(uh, new, color, &key);
            families[0].2.push((key.as_ref().to_vec(), buf));
        }
        for (color, bucket, mut group) in placed.cold {
            let mut buf = Vec::new();
            group.pack_entire(&mut buf);
            let key = ColdGroupKey::new(uh, new, color, bucket);
            families[1].2.push((key.as_ref().to_vec(), buf));
        }
        for (_, _, entries) in &families {
            debug_assert!(entries.is_sorted_by(|a, b| a.0 < b.0));
            stats.bytes += entries
                .iter()
                .map(|(key, value)| (key.len() + value.len()) as u64)
                .sum::<u64>();
        }
        stats.warm_entries = families[0].2.len() as u64;
        stats.cold_groups = families[1].2.len() as u64;

        if stats.bytes > self.config.ingest_bytes {
            stats.ingested = true;
            let dir = self.path.join(REBUILD_DIR);
            fs::create_dir_all(&dir)?;
            for (cf, opts, entries) in &families {
                if entries.is_empty() {
                    continue;
                }
                let path = dir.join(format!("{cf}_{uh}_{}.sst", new.0));
                let mut writer = SstFileWriter::create(opts);
                writer.open(&path)?;
                for (key, value) in entries {
                    writer.put(key, value)?;
                }
                writer.finish()?;
                let mut ingest_opts = IngestExternalFileOptions::default();
                ingest_opts.set_move_files(true);
                self.db
                    .ingest_external_file_cf_opts(self.cf(cf), &ingest_opts, vec![path])?;
            }
        } else {
            let mut batch = WriteBatch::default();
            for (cf, _, entries) in &families {
                for (key, value) in entries {
                    batch.put_cf(self.cf(cf), key, value);
                    if batch.size_in_bytes() >= self.config.batch_bytes {
                        self.db.write(mem::take(&mut batch))?;
                    }
                }
            }
            if !batch.is_empty() {
                self.db.write(batch)?;
            }
        }

        // Switch to the new generation and drop the old ones.
        status.generation = new;
        status.cold_bits = cold_bits;
        status.games_at_rebuild = status.games;
        let mut batch = WriteBatch::default();
        let mut buf = Vec::new();
        status.pack(&mut buf);
        batch.put_cf(self.cf(STATUS_CF), StatusKey::new(uh), &buf);
        let (start, end) = uh.generations_range(Generation(0), Some(new));
        batch.delete_range_cf(self.cf(WARM_CF), &start, &end);
        batch.delete_range_cf(self.cf(COLD_CF), &start, &end);
        self.db.write(batch)?;

        let c = &self.counters;
        c.rebuilds.inc();
        c.rebuilds_ingested.add(u64::from(stats.ingested));
        c.rebuild_bytes.add(stats.bytes);
        c.rebuild_micros
            .add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        Ok(Some(stats))
    }

    /// Options of a column family for writing SST files to ingest (for the
    /// bulk build), with their own small block cache.
    pub fn sst_options(cf: &str) -> Options {
        cf_options(cf, &Cache::new_lru_cache(8 << 20))
    }

    /// Ingests SST files into a column family, moving (hard linking) them.
    /// Files that overlap nothing go to the bottom level.
    pub fn ingest(&self, cf: &str, files: Vec<PathBuf>) -> Result<(), PlayerDbError> {
        let mut ingest_opts = IngestExternalFileOptions::default();
        ingest_opts.set_move_files(true);
        self.db
            .ingest_external_file_cf_opts(self.cf(cf), &ingest_opts, files)?;
        Ok(())
    }

    /// Creates a checkpoint (hard links of the SST files) in a new
    /// directory.
    pub fn checkpoint(&self, path: &Path) -> Result<(), PlayerDbError> {
        rocksdb::checkpoint::Checkpoint::new(&self.db)?.create_checkpoint(path)?;
        Ok(())
    }

    /// Bytes of the live SST files of a column family, per level.
    pub fn level_bytes(&self, cf: &str) -> Result<Vec<u64>, PlayerDbError> {
        let mut levels = vec![0; NUM_LEVELS];
        for file in self.db.live_files()? {
            if file.column_family_name == cf {
                levels[(file.level.max(0) as usize).min(NUM_LEVELS - 1)] += file.size as u64;
            }
        }
        Ok(levels)
    }

    /// With several `db_paths`, RocksDB ignores
    /// `level_compaction_dynamic_level_bytes` (only logging a warning) and
    /// sizes the levels statically, from `max_bytes_for_level_base`, which
    /// lets the last level grow far beyond 10x its upper level (write
    /// amplification ~90 in the old production database). This emulates
    /// dynamic sizing: per column family, the level above the last gets a
    /// tenth (`level_ratio`) of the last level, with the levels above in a
    /// geometric series from `LEVEL_BASE`.
    ///
    /// RocksDB picks the path of a compaction output from the level
    /// targets, not the actual level sizes. So the target of the last level
    /// is raised to at least its actual size, or the whole last level would
    /// stay on the first path long after it outgrew it.
    ///
    /// Called at open and periodically, as the database grows. Does nothing
    /// with a single path.
    pub fn tune_levels(&self) -> Result<(), PlayerDbError> {
        if self.config.db_paths.len() < 2 {
            return Ok(());
        }
        for cf in ALL_CFS {
            let levels = self.level_bytes(cf)?;
            let last = levels[NUM_LEVELS - 1];
            let levels = static_levels(last, self.config.level_ratio);
            let additional = format!("1:1:1:1:1:{}", levels.last_additional);
            self.db.set_options_cf(
                self.cf(cf),
                &[
                    ("max_bytes_for_level_base", &levels.base.to_string()),
                    (
                        "max_bytes_for_level_multiplier",
                        &format!("{:.3}", levels.multiplier),
                    ),
                    ("max_bytes_for_level_multiplier_additional", &additional),
                ],
            )?;
            log::info!(
                "{cf}: last level {:.1} GiB, level base {} MiB, multiplier {:.2}, last additional {} (level {} target {:.1} GiB, level {} target {:.1} GiB)",
                gib(last as f64),
                levels.base >> 20,
                levels.multiplier,
                levels.last_additional,
                NUM_LEVELS - 2,
                gib(levels.target(NUM_LEVELS - 2)),
                NUM_LEVELS - 1,
                gib(levels.target(NUM_LEVELS - 1)),
            );
        }
        Ok(())
    }

    /// Waits until no flush or compaction is pending or running.
    pub fn wait_for_compactions(&self) -> Result<(), PlayerDbError> {
        let mut opts = rocksdb::WaitForCompactOptions::default();
        opts.set_flush(true);
        self.db.wait_for_compact(&opts)?;
        Ok(())
    }

    /// Deletes all data of a player.
    pub fn delete_player(&self, uh: UserHash) -> Result<(), PlayerDbError> {
        let (start, end) = uh.range();
        let mut batch = WriteBatch::default();
        for cf in PLAYER_CFS {
            batch.delete_range_cf(self.cf(cf), &start, &end);
        }
        self.db.write(batch)?;
        Ok(())
    }

    /// A consistent view for reading.
    pub fn reader(&self) -> PlayerReader<'_> {
        PlayerReader {
            db: self,
            snapshot: self.db.snapshot(),
        }
    }

    pub fn cache_usage(&self) -> usize {
        self.cache.get_usage()
    }

    pub fn ticker(&self, ticker: Ticker) -> Option<u64> {
        self.config
            .statistics
            .then(|| self.opts.get_ticker_count(ticker))
    }

    pub fn property(&self, name: &str) -> Result<Option<String>, rocksdb::Error> {
        self.db.property_value(name)
    }

    pub fn cf_property(&self, cf: &str, name: &str) -> Result<Option<String>, rocksdb::Error> {
        match self.db.cf_handle(cf) {
            Some(cf) => self.db.property_value_cf(cf, name),
            None => Ok(None),
        }
    }

    pub fn cf_int_property(&self, cf: &str, name: &str) -> Result<Option<u64>, rocksdb::Error> {
        match self.db.cf_handle(cf) {
            Some(cf) => self.db.property_int_value_cf(cf, name),
            None => Ok(None),
        }
    }

    /// The SST files of all column families, with their levels and sizes.
    pub fn live_files(&self) -> Result<Vec<LiveFile>, rocksdb::Error> {
        self.db.live_files()
    }

    pub fn column_families() -> [&'static str; 6] {
        ALL_CFS
    }

    /// Flushes the memtables (for tests and measurements).
    pub fn flush(&self) -> Result<(), PlayerDbError> {
        for cf in ALL_CFS {
            self.db.flush_cf(self.cf(cf))?;
        }
        Ok(())
    }

    /// The generations of the warm and cold keys of a player.
    #[cfg(test)]
    pub fn index_generations(
        &self,
        uh: UserHash,
    ) -> Result<std::collections::BTreeSet<Generation>, PlayerDbError> {
        let (start, end) = uh.range();
        let mut generations = std::collections::BTreeSet::new();
        for cf in [WARM_CF, COLD_CF] {
            let mut read_opts = ReadOptions::default();
            read_opts.set_iterate_lower_bound(start.clone());
            read_opts.set_iterate_upper_bound(end.clone());
            for item in self.db.iterator_cf_opt(
                self.cf(cf),
                read_opts,
                IteratorMode::From(&start, Direction::Forward),
            ) {
                let (key, _) = item?;
                generations.insert(Generation(u32::from_be_bytes(
                    key[UserHash::SIZE..UserHash::SIZE + 4]
                        .try_into()
                        .expect("generation"),
                )));
            }
        }
        Ok(generations)
    }

    /// Calls `f` with the keys and (merged) values of a column family in
    /// `[start, end)`, without filling the block cache (for measurements).
    pub fn scan(
        &self,
        cf: &str,
        start: &[u8],
        end: &[u8],
        mut f: impl FnMut(&[u8], &[u8]),
    ) -> Result<(), PlayerDbError> {
        let mut read_opts = ReadOptions::default();
        read_opts.set_iterate_lower_bound(start.to_vec());
        read_opts.set_iterate_upper_bound(end.to_vec());
        read_opts.fill_cache(false);
        for item in self.db.iterator_cf_opt(
            self.cf(cf),
            read_opts,
            IteratorMode::From(start, Direction::Forward),
        ) {
            let (key, value) = item?;
            f(&key, &value);
        }
        Ok(())
    }

    /// Compacts everything (for tests and measurements).
    pub fn compact(&self) {
        for cf in ALL_CFS {
            self.db
                .compact_range_cf(self.cf(cf), None::<&[u8]>, None::<&[u8]>);
        }
    }
}

/// Levels of the column families (RocksDB's default).
const NUM_LEVELS: usize = 7;

/// Target of level 1 for static level sizes.
const LEVEL_BASE: u64 = 128 << 20;

/// Static level sizes for a last level of `last` bytes.
#[derive(Debug, PartialEq)]
struct StaticLevels {
    base: u64,
    /// Such that the level above the last has `1 / ratio` of it.
    multiplier: f64,
    /// Extra factor of the last level, such that its target is at least its
    /// actual size. Only used to place it in `db_paths`, because RocksDB
    /// never compacts the last level for its size.
    last_additional: u32,
}

impl StaticLevels {
    /// Target size of a level, as RocksDB computes it (also for level 0).
    fn target(&self, level: usize) -> f64 {
        let mut target = self.base as f64 * self.multiplier.powi(level.max(1) as i32 - 1);
        if level == NUM_LEVELS - 1 {
            target *= f64::from(self.last_additional);
        }
        target
    }
}

fn static_levels(last: u64, ratio: f64) -> StaticLevels {
    let upper = last as f64 / ratio;
    // Levels 1 ..= NUM_LEVELS - 2.
    let steps = (NUM_LEVELS - 3) as f64;
    let multiplier = (upper / LEVEL_BASE as f64)
        .powf(1.0 / steps)
        .clamp(2.0, 10.0);
    let unscaled_last = LEVEL_BASE as f64 * multiplier.powi(NUM_LEVELS as i32 - 2);
    StaticLevels {
        base: LEVEL_BASE,
        multiplier,
        // Saturating float to int conversion.
        last_additional: ((last as f64 / unscaled_last).ceil() as u32).max(1),
    }
}

/// Relative column family sizes estimated from simulating a production-like
/// database.
fn path_share(cf: &str) -> f64 {
    match cf {
        COLD_CF => 0.44,
        WARM_CF => 0.29,
        GAMES_CF => 0.22,
        GAME_IDS_CF => 0.04,
        STATUS_CF => 0.01,
        // Only the user hash key.
        _ => 0.0,
    }
}

fn db_paths(paths: &[(PathBuf, u64)], share: f64) -> Result<Vec<DBPath>, rocksdb::Error> {
    paths
        .iter()
        .map(|(path, size)| DBPath::new(path, (*size as f64 * share) as u64))
        .collect()
}

fn gib(bytes: f64) -> f64 {
    bytes / (1u64 << 30) as f64
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PositionStats {
    pub warm_hit: bool,
    pub cold_candidates: u64,
    pub cold_validated: u64,
}

/// Reads of a consistent snapshot of the database.
pub struct PlayerReader<'a> {
    db: &'a PlayerDb,
    snapshot: SnapshotWithThreadMode<'a, DB>,
}

impl PlayerReader<'_> {
    pub fn status(&self, uh: UserHash) -> Result<Option<PlayerStatus>, PlayerDbError> {
        Ok(self
            .snapshot
            .get_pinned_cf(self.db.cf(STATUS_CF), StatusKey::new(uh))?
            .map(|value| PlayerStatus::unpack(&value)))
    }

    /// The games of the player that reached a position, from the warm entry
    /// and the validated candidates of the cold group, as one entry.
    pub fn position(
        &self,
        status: &PlayerStatus,
        color: Color,
        pos: &VariantPosition,
    ) -> Result<(PlayerEntry, PositionStats), PlayerDbError> {
        let uh = self.db.user_hash(&status.user);
        let key = PositionKey::new(pos);
        let mut stats = PositionStats::default();
        let mut entry = PlayerEntry::new();
        if let Some(warm) = self.snapshot.get_pinned_cf(
            self.db.cf(WARM_CF),
            WarmKey::new(uh, status.generation, color, &key),
        )? {
            entry.merge_unpack_entire(&warm);
            stats.warm_hit = true;
        }

        let (bucket, suffix) = ColdBucket::of(&key, *status.cold_bits.get(color));
        let candidates: Vec<PlayerGameIndex> = match self.snapshot.get_pinned_cf(
            self.db.cf(COLD_CF),
            ColdGroupKey::new(uh, status.generation, color, bucket),
        )? {
            Some(group) => ColdGroupView::unpack_entire(&group)
                .candidates(suffix)
                .map(|index| {
                    PlayerGameIndex(u32::try_from(index).expect("valid player game index"))
                })
                .collect(),
            None => Vec::new(),
        };
        stats.cold_candidates = candidates.len() as u64;
        if !candidates.is_empty() {
            for (&index, game) in candidates.iter().zip(self.game_values(uh, &candidates)?) {
                let Some(game) = game else {
                    log::warn!("cold candidate {index} of {} not found", status.user);
                    continue;
                };
                let game = BorrowedPlayerGame::unpack(&game);
                if let Some(m) = find_visit(&game, &key, MAX_PLY) {
                    entry.add_game(
                        CellKey::new(
                            m,
                            PlayerGroupHeader {
                                speed: game.speed,
                                mode: game.mode,
                            },
                            game.month,
                        ),
                        game.opponent_rating,
                        game.outcome,
                        index.into(),
                    );
                    stats.cold_validated += 1;
                }
            }
            let c = &self.db.counters;
            c.cold_candidates.add(stats.cold_candidates);
            c.cold_false
                .add(stats.cold_candidates - stats.cold_validated);
        }
        Ok((entry, stats))
    }

    fn game_values(
        &self,
        uh: UserHash,
        indexes: &[PlayerGameIndex],
    ) -> Result<Vec<Option<Vec<u8>>>, PlayerDbError> {
        let cf = self.db.cf(GAMES_CF);
        self.snapshot
            .multi_get_cf(indexes.iter().map(|&index| (cf, GameKey::new(uh, index))))
            .into_iter()
            .map(|result| result.map_err(PlayerDbError::from))
            .collect()
    }

    /// Games of a player, in the order of the indexes.
    pub fn games(
        &self,
        uh: UserHash,
        indexes: &[PlayerGameIndex],
    ) -> Result<Vec<Option<PlayerGame>>, PlayerDbError> {
        Ok(self
            .game_values(uh, indexes)?
            .into_iter()
            .map(|value| value.map(|value| BorrowedPlayerGame::unpack(&value).into_owned()))
            .collect())
    }
}

#[cfg(test)]
mod level_tests {
    use super::*;

    #[test]
    fn test_static_levels() {
        // A tenth of the last level in level 5, levels 1 ..= 5 geometric.
        let levels = static_levels(340_000_000_000, 10.0);
        let level5 = levels.target(5);
        assert!((level5 / 34e9 - 1.0).abs() < 0.01, "{level5}");
        // Small databases: the smallest multiplier.
        assert_eq!(
            static_levels(50_000_000, 10.0),
            StaticLevels {
                base: LEVEL_BASE,
                multiplier: 2.0,
                last_additional: 1
            }
        );
        // Huge ones: at most 10.
        assert_eq!(static_levels(u64::MAX / 2, 10.0).multiplier, 10.0);
    }

    #[test]
    fn test_last_level_target_covers_last_level() {
        for last in [
            0,
            1 << 30,
            20 << 30,
            21 << 30,
            100 << 30,
            340_000_000_000,
            5 << 40,
        ] {
            for ratio in [2.0, 10.0, 16.0] {
                let levels = static_levels(last, ratio);
                let target = levels.target(NUM_LEVELS - 1);
                let step = target / f64::from(levels.last_additional);
                assert!(target >= last as f64, "{last} {ratio}");
                assert!(
                    levels.last_additional == 1 || target < last as f64 + step,
                    "{last} {ratio}"
                );
            }
        }
    }

    #[test]
    fn test_path_shares() {
        let total: f64 = ALL_CFS.into_iter().map(path_share).sum();
        assert!((total - 1.0).abs() < 1e-9, "{total}");
    }
}
