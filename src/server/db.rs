use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

use rocksdb::{
    BlockBasedTablePinningTier, Cache, ColumnFamily, ColumnFamilyDescriptor, DB, DBPinnableSlice,
    ErrorKind, IngestExternalFileOptions, Options, ReadOptions, ReadTier,
    statistics::{StatsLevel, Ticker},
};
use thiserror::Error;

use crate::{
    packed::{ColdGroupView, ColdKey, GameId, GameIndex, PositionKey},
    sst::SstConfig,
};

pub const GAMES_CF: &str = "games";
pub const POSITIONS_CF: &str = "positions";
pub const COLD_CF: &str = "cold_positions";
/// Optional map from `GameId` to `GameIndex`, for looking up games by id.
pub const GAME_IDS_CF: &str = "game_ids";

/// Subdirectory of the database for hard links of SST files that are about
/// to be ingested.
const STAGING_DIR: &str = "staging";

#[derive(Debug, Clone, Default)]
pub struct DbConfig {
    /// Size of the shared block cache in bytes.
    pub cache_bytes: u64,
    /// Bypass the page cache.
    pub direct_reads: bool,
    /// Collect RocksDB statistics.
    pub statistics: bool,
    /// Threads for opening table files at startup.
    pub file_opening_threads: usize,
    /// Load all index and filter partitions when opening, and pin them in
    /// the block cache.
    pub pin_metadata: bool,
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("rocksdb: {0}")]
    RocksDb(#[from] rocksdb::Error),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("{0} already exists")]
    Exists(PathBuf),
    #[error("missing directory {0}")]
    MissingDir(PathBuf),
}

fn descriptors(cache: &Cache, pin_metadata: bool) -> Vec<ColumnFamilyDescriptor> {
    [GAMES_CF, POSITIONS_CF, COLD_CF, GAME_IDS_CF]
        .into_iter()
        .map(|name| {
            let mut table_opts = SstConfig::for_cf(name)
                .expect("explorer cf")
                .read_table_options();
            table_opts.set_block_cache(cache);
            table_opts.set_cache_index_and_filter_blocks(true);
            table_opts.set_cache_index_and_filter_blocks_with_high_priority(true);
            table_opts.set_pin_top_level_index_and_filter(true);
            if pin_metadata {
                table_opts.set_top_level_index_pinning_tier(BlockBasedTablePinningTier::All);
                table_opts.set_partition_pinning_tier(BlockBasedTablePinningTier::All);
                table_opts.set_unpartitioned_pinning_tier(BlockBasedTablePinningTier::All);
            }
            let mut opts = Options::default();
            opts.set_block_based_table_factory(&table_opts);
            opts.set_disable_auto_compactions(true);
            ColumnFamilyDescriptor::new(name, opts)
        })
        .collect()
}

fn db_options(config: &DbConfig) -> Options {
    let mut opts = Options::default();
    opts.set_max_open_files(-1);
    opts.set_max_file_opening_threads(config.file_opening_threads.clamp(1, 64) as i32);
    opts.set_use_direct_reads(config.direct_reads);
    opts.set_skip_stats_update_on_db_open(true);
    opts.set_table_cache_num_shard_bits(6);
    if config.statistics {
        opts.enable_statistics();
        opts.set_statistics_level(StatsLevel::ExceptDetailedTimers);
    }
    opts
}

/// An LRU cache in which index and filter blocks (inserted with high
/// priority) are only evicted by each other, never by data blocks. A hyper
/// clock cache has no such reserved pool, and caused many more disk reads.
fn make_cache(config: &DbConfig) -> Cache {
    let mut opts = rocksdb::LruCacheOptions::default();
    opts.set_capacity(config.cache_bytes as usize);
    opts.set_high_pri_pool_ratio(1.0);
    Cache::new_lru_cache_opts(&opts)
}

/// SST files of a directory, in key order (file names sort by key range).
fn sst_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .filter(|path| {
            path.as_ref()
                .is_ok_and(|path| path.extension().is_some_and(|ext| ext == "sst"))
        })
        .collect::<io::Result<_>>()?;
    files.sort();
    Ok(files)
}

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub games_files: usize,
    pub positions_files: usize,
    pub cold_files: usize,
    pub game_ids_files: usize,
}

/// Creates a database at `path` from an SST directory, laid out like the
/// output of parquet_to_games and parquet_to_positions: the subdirectories
/// `games`, `positions`, `cold_positions` (unless `cold` is false) and,
/// if it exists, `game_ids`. See [`ingest`].
pub fn ingest_dir(path: &Path, sst: &Path, cold: bool) -> Result<IngestStats, DbError> {
    let required = |name: &str| {
        let dir = sst.join(name);
        if dir.is_dir() {
            Ok(dir)
        } else {
            Err(DbError::MissingDir(dir))
        }
    };
    let games = required("games")?;
    let positions = required("positions")?;
    let cold = cold.then(|| required("cold_positions")).transpose()?;
    let game_ids = Some(sst.join("game_ids")).filter(|dir| dir.is_dir());
    ingest(
        path,
        &games,
        &positions,
        cold.as_deref(),
        game_ids.as_deref(),
    )
}

/// Creates a database at `path` from the SST files of the directories,
/// which are hard-linked (the originals are left untouched). The
/// directories must be on the same file system as the database. Without
/// `cold` or `game_ids`, their column families stay empty.
///
/// The database is built next to `path`, in `path` with the extension
/// `.tmp` (removed first if left over from an interrupted run), and renamed
/// to `path` once complete, so `path` exists only as a complete database.
pub fn ingest(
    path: &Path,
    games: &Path,
    positions: &Path,
    cold: Option<&Path>,
    game_ids: Option<&Path>,
) -> Result<IngestStats, DbError> {
    if path.exists() {
        return Err(DbError::Exists(path.to_owned()));
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if tmp.exists() {
        log::warn!("removing {} of an interrupted ingest", tmp.display());
        fs::remove_dir_all(&tmp)?;
    }
    let stats = ingest_into(&tmp, games, positions, cold, game_ids)?;
    fs::rename(&tmp, path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::File::open(parent)?.sync_all()?;
    Ok(stats)
}

fn ingest_into(
    path: &Path,
    games: &Path,
    positions: &Path,
    cold: Option<&Path>,
    game_ids: Option<&Path>,
) -> Result<IngestStats, DbError> {
    let config = DbConfig {
        cache_bytes: 64 << 20,
        ..DbConfig::default()
    };
    let cache = make_cache(&config);
    let mut opts = db_options(&config);
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    // With an unlimited table cache, RocksDB opens a table reader for every
    // ingested file (footer, properties, index and filter) when it applies
    // the version edit; with a limited one it only opens a few.
    opts.set_max_open_files(64);
    let db = DB::open_cf_descriptors(&opts, path, descriptors(&cache, false))?;
    let staging = path.join(STAGING_DIR);
    fs::create_dir_all(&staging)?;

    let mut stats = IngestStats::default();
    for (cf_name, dir, count) in [
        (GAMES_CF, Some(games), &mut stats.games_files),
        (POSITIONS_CF, Some(positions), &mut stats.positions_files),
        (COLD_CF, cold, &mut stats.cold_files),
        (GAME_IDS_CF, game_ids, &mut stats.game_ids_files),
    ] {
        let Some(dir) = dir else {
            continue;
        };
        let started = Instant::now();
        let files = sst_files(dir)?;
        // RocksDB removes the "moved" files after linking them into the
        // database, so link into a staging directory first.
        let staged: Vec<PathBuf> = files
            .iter()
            .map(|file| {
                let staged = staging.join(format!(
                    "{cf_name}_{}",
                    file.file_name().expect("file name").to_string_lossy()
                ));
                fs::hard_link(file, &staged)?;
                Ok::<_, io::Error>(staged)
            })
            .collect::<Result<_, _>>()?;
        let mut ingest_opts = IngestExternalFileOptions::default();
        ingest_opts.set_move_files(true);
        ingest_opts.set_snapshot_consistency(false);
        ingest_opts.set_allow_blocking_flush(false);
        db.ingest_external_file_cf_opts(
            db.cf_handle(cf_name).expect("column family"),
            &ingest_opts,
            staged,
        )?;
        *count = files.len();
        log::info!(
            "ingested {} files into {cf_name} in {:.1?}",
            files.len(),
            started.elapsed()
        );
    }
    fs::remove_dir(&staging)?;
    Ok(stats)
}

/// A read-only view of an ingested database.
pub struct Database {
    db: DB,
    cache: Cache,
    opts: Options,
    statistics: bool,
}

impl Database {
    pub fn open_read_only(path: &Path, config: &DbConfig) -> Result<Database, DbError> {
        let started = Instant::now();
        let cache = make_cache(config);
        let opts = db_options(config);
        let db = DB::open_cf_descriptors_read_only(
            &opts,
            path,
            descriptors(&cache, config.pin_metadata),
            false,
        )?;
        log::info!(
            "opened {} read-only in {:.1?}",
            path.display(),
            started.elapsed()
        );
        Ok(Database {
            db,
            cache,
            opts,
            statistics: config.statistics,
        })
    }

    fn cf(&self, name: &str) -> &ColumnFamily {
        self.db.cf_handle(name).expect("column family")
    }

    /// The packed `LichessEntry` of a position, if it was reached by more
    /// than one game.
    pub fn positions_entry(
        &self,
        hash: &PositionKey,
    ) -> Result<Option<DBPinnableSlice<'_>>, rocksdb::Error> {
        self.db
            .get_pinned_cf(self.cf(POSITIONS_CF), hash.as_bytes())
    }

    /// Games that may have reached the position exactly once (to be
    /// validated by replaying them).
    pub fn cold_candidates(&self, hash: &PositionKey) -> Result<Vec<GameIndex>, rocksdb::Error> {
        let key = ColdKey::new(hash);
        Ok(self
            .db
            .get_pinned_cf(self.cf(COLD_CF), key.prefix())?
            .map_or_else(Vec::new, |group| {
                ColdGroupView::unpack_entire(&group)
                    .candidates(key.suffix())
                    .map(|index| GameIndex::new(index).expect("valid game index"))
                    .collect()
            }))
    }

    /// The value of a game (see `unpack_game_value`).
    pub fn game(&self, index: GameIndex) -> Result<Option<DBPinnableSlice<'_>>, rocksdb::Error> {
        self.db.get_pinned_cf(self.cf(GAMES_CF), index.to_bytes())
    }

    /// The index of a game, if the database has the map of game ids.
    pub fn game_index(&self, id: GameId) -> Result<Option<GameIndex>, rocksdb::Error> {
        Ok(self
            .db
            .get_pinned_cf(self.cf(GAME_IDS_CF), id.to_bytes())?
            .map(|value| GameIndex::unpack(&mut &value[..])))
    }

    /// Values of games, in the order of the indexes (`None` for unknown
    /// indexes).
    pub fn games(
        &self,
        indexes: &[GameIndex],
    ) -> Result<Vec<Option<DBPinnableSlice<'_>>>, rocksdb::Error> {
        let keys: Vec<[u8; GameIndex::SIZE]> =
            indexes.iter().map(|index| index.to_bytes()).collect();
        self.db
            .batched_multi_get_cf_opt(self.cf(GAMES_CF), &keys, false, &ReadOptions::default())
            .into_iter()
            .collect()
    }

    /// Like `games()`, but only from the block cache: `None` for games
    /// that would have to be read from disk.
    #[allow(clippy::type_complexity)]
    pub fn cached_games(
        &self,
        indexes: &[GameIndex],
    ) -> Result<Vec<Option<Option<DBPinnableSlice<'_>>>>, rocksdb::Error> {
        let keys: Vec<[u8; GameIndex::SIZE]> =
            indexes.iter().map(|index| index.to_bytes()).collect();
        let mut opts = ReadOptions::default();
        opts.set_read_tier(ReadTier::BlockCache);
        self.db
            .batched_multi_get_cf_opt(self.cf(GAMES_CF), &keys, false, &opts)
            .into_iter()
            .map(|result| match result {
                Ok(game) => Ok(Some(game)),
                Err(err) if err.kind() == ErrorKind::Incomplete => Ok(None),
                Err(err) => Err(err),
            })
            .collect()
    }

    pub fn cache_usage(&self) -> usize {
        self.cache.get_usage()
    }

    pub fn cache_pinned_usage(&self) -> usize {
        self.cache.get_pinned_usage()
    }

    /// The count of a ticker, if statistics are collected.
    pub fn ticker(&self, ticker: Ticker) -> Option<u64> {
        self.statistics.then(|| self.opts.get_ticker_count(ticker))
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
}
