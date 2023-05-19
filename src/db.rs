use std::{path::PathBuf, time::Instant};

use clap::Parser;
use rocksdb::{
    properties::{ESTIMATE_NUM_KEYS, OPTIONS_STATISTICS},
    BlockBasedOptions, Cache, ColumnFamily, ColumnFamilyDescriptor, DBCompressionType,
    MergeOperands, Options, ReadOptions, SliceTransform, WriteBatch, DB,
};

use crate::{
    api::{HistoryWanted, LichessQueryFilter, Limits},
    model::{
        GameId, History, HistoryBuilder, Key, KeyPrefix, LichessEntry, LichessGame, MastersEntry,
        MastersGame, Month, PlayerEntry, PlayerStatus, PreparedResponse, UserId, Year,
    },
};

#[derive(Parser)]
pub struct DbOpt {
    /// Path to RocksDB database.
    #[arg(long, default_value = "_db")]
    db: PathBuf,
    /// Tune compaction readahead for spinning disks.
    #[arg(long)]
    db_compaction_readahead: bool,
    /// Size of RocksDB block cache in bytes. Use around 2/3 of the systems
    /// RAM, leaving some memory for the operating system page cache.
    #[arg(long, default_value = "4294967296")]
    db_cache: usize,
    /// Rate limits for writes to disk in bytes per second. This is used to
    /// limit the speed of indexing and importing (flushes and compactions),
    /// so that enough bandwidth remains to respond to queries. Use a sustained
    /// rate that your disks can comfortably handle.
    #[arg(long, default_value = "10485760")]
    db_rate_limit: i64,
}

#[derive(Default)]
pub struct DbStats {
    pub block_index_miss: u64,
    pub block_index_hit: u64,
    pub block_filter_miss: u64,
    pub block_filter_hit: u64,
    pub block_data_miss: u64,
    pub block_data_hit: u64,
}

impl DbStats {
    fn read_options_statistics(&mut self, s: &str) {
        fn count(line: &str, prefix: &str) -> Option<u64> {
            line.strip_prefix(prefix)
                .and_then(|suffix| suffix.strip_prefix(" COUNT : "))
                .and_then(|suffix| suffix.parse().ok())
        }

        for line in s.lines() {
            if let Some(c) = count(line, "rocksdb.block.cache.index.miss") {
                self.block_index_miss = c;
            } else if let Some(c) = count(line, "rocksdb.block.cache.index.hit") {
                self.block_index_hit = c;
            } else if let Some(c) = count(line, "rocksdb.block.cache.filter.miss") {
                self.block_filter_miss = c;
            } else if let Some(c) = count(line, "rocksdb.block.cache.filter.hit") {
                self.block_filter_hit = c;
            } else if let Some(c) = count(line, "rocksdb.block.cache.data.miss") {
                self.block_data_miss = c;
            } else if let Some(c) = count(line, "rocksdb.block.cache.data.hit") {
                self.block_data_hit = c;
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct CacheHint {
    ply: u32,
}

impl CacheHint {
    pub fn from_ply(ply: u32) -> CacheHint {
        CacheHint { ply }
    }

    pub fn always() -> CacheHint {
        CacheHint { ply: 0 }
    }

    pub fn should_fill_cache(&self) -> bool {
        let percent = if self.ply < 5 {
            return true;
        } else if self.ply < 10 {
            90
        } else if self.ply < 15 {
            70
        } else if self.ply < 20 {
            40
        } else if self.ply < 25 {
            10
        } else {
            2
        };
        fastrand::u32(0..100) < percent
    }
}

// Note on usage in async contexts: All database operations are blocking
// (https://github.com/facebook/rocksdb/issues/3254). Calls should be run in a
// thread-pool to avoid blocking other requests.
pub struct Database {
    pub inner: DB,
}

type MergeFn = fn(key: &[u8], existing: Option<&[u8]>, operands: &MergeOperands) -> Option<Vec<u8>>;

struct Column<'a> {
    name: &'a str,
    prefix: Option<usize>,
    merge: Option<(&'a str, MergeFn)>,
    cache: &'a Cache,
}

impl Column<'_> {
    fn descriptor(self) -> ColumnFamilyDescriptor {
        // Mostly using modern defaults from
        // https://github.com/facebook/rocksdb/wiki/Setup-Options-and-Basic-Tuning.
        let mut table_opts = BlockBasedOptions::default();
        table_opts.set_block_cache(self.cache);
        table_opts.set_block_size(64 * 1024); // Spinning disks
        table_opts.set_cache_index_and_filter_blocks(true);
        table_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
        table_opts.set_hybrid_ribbon_filter(10.0, 1);
        table_opts.set_whole_key_filtering(self.prefix.is_none()); // Only prefix seeks for positions
        table_opts.set_format_version(5);

        let mut cf_opts = Options::default();
        cf_opts.set_block_based_table_factory(&table_opts);
        cf_opts.set_compression_type(DBCompressionType::Lz4);
        cf_opts.set_bottommost_compression_type(DBCompressionType::Zstd);
        cf_opts.set_level_compaction_dynamic_level_bytes(false); // Infinitely growing database

        cf_opts.set_prefix_extractor(match self.prefix {
            Some(prefix) => SliceTransform::create_fixed_prefix(prefix),
            None => SliceTransform::create_noop(),
        });

        if let Some((name, merge_fn)) = self.merge {
            cf_opts.set_merge_operator_associative(name, merge_fn);
        }

        ColumnFamilyDescriptor::new(self.name, cf_opts)
    }
}

impl Database {
    pub fn open(opt: DbOpt) -> Result<Database, rocksdb::Error> {
        let started_at = Instant::now();

        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        db_opts.set_max_background_jobs(if opt.db_compaction_readahead { 2 } else { 4 });
        db_opts.set_ratelimiter(opt.db_rate_limit, 100_000, 10);
        db_opts.set_write_buffer_size(128 * 1024 * 1024); // bulk loads

        db_opts.enable_statistics();

        if opt.db_compaction_readahead {
            db_opts.set_compaction_readahead_size(2 * 1024 * 1024);
        }

        let cache = Cache::new_lru_cache(opt.db_cache);

        let inner = DB::open_cf_descriptors(
            &db_opts,
            opt.db,
            vec![
                // Masters database
                Column {
                    name: "masters",
                    prefix: Some(KeyPrefix::SIZE),
                    merge: Some(("masters_merge", masters_merge)),
                    cache: &cache,
                }
                .descriptor(),
                Column {
                    name: "masters_game",
                    prefix: None,
                    merge: None,
                    cache: &cache,
                }
                .descriptor(),
                // Lichess database
                Column {
                    name: "lichess",
                    prefix: Some(KeyPrefix::SIZE),
                    merge: Some(("lichess_merge", lichess_merge)),
                    cache: &cache,
                }
                .descriptor(),
                Column {
                    name: "lichess_game",
                    prefix: None,
                    merge: Some(("lichess_game_merge", lichess_game_merge)),
                    cache: &cache,
                }
                .descriptor(),
                // Player database (also shares lichess_game)
                Column {
                    name: "player",
                    prefix: Some(KeyPrefix::SIZE),
                    merge: Some(("player_merge", player_merge)),
                    cache: &cache,
                }
                .descriptor(),
                Column {
                    name: "player_status",
                    prefix: None,
                    merge: None,
                    cache: &cache,
                }
                .descriptor(),
            ],
        )?;

        let elapsed = started_at.elapsed();
        log::info!("database opened in {elapsed:.3?}");

        Ok(Database { inner })
    }

    pub fn stats(&self) -> Result<DbStats, rocksdb::Error> {
        let mut stats = DbStats::default();
        if let Some(options_statistics) = self.inner.property_value(OPTIONS_STATISTICS)? {
            stats.read_options_statistics(&options_statistics);
        }
        Ok(stats)
    }

    pub fn compact(&self) {
        self.lichess().compact();
        self.masters().compact();
        log::info!("finished manual compaction");
    }

    pub fn masters(&self) -> MastersDatabase<'_> {
        MastersDatabase {
            inner: &self.inner,
            cf_masters: self.inner.cf_handle("masters").expect("cf masters"),
            cf_masters_game: self
                .inner
                .cf_handle("masters_game")
                .expect("cf masters_game"),
        }
    }

    pub fn lichess(&self) -> LichessDatabase<'_> {
        LichessDatabase {
            inner: &self.inner,
            cf_lichess: self.inner.cf_handle("lichess").expect("cf lichess"),
            cf_lichess_game: self
                .inner
                .cf_handle("lichess_game")
                .expect("cf lichess_game"),

            cf_player: self.inner.cf_handle("player").expect("cf player"),
            cf_player_status: self
                .inner
                .cf_handle("player_status")
                .expect("cf player_status"),
        }
    }
}

pub struct MastersDatabase<'a> {
    inner: &'a DB,
    cf_masters: &'a ColumnFamily,
    cf_masters_game: &'a ColumnFamily,
}

pub struct MastersStats {
    pub num_masters: u64,
    pub num_masters_game: u64,
}

impl MastersDatabase<'_> {
    pub fn compact(&self) {
        log::info!("running manual compaction for masters ...");
        compact_column(self.inner, self.cf_masters);
        log::info!("running manual compaction for masters_game ...");
        compact_column(self.inner, self.cf_masters_game);
    }

    pub fn estimate_stats(&self) -> Result<MastersStats, rocksdb::Error> {
        Ok(MastersStats {
            num_masters: self
                .inner
                .property_int_value_cf(self.cf_masters, ESTIMATE_NUM_KEYS)?
                .unwrap_or(0),
            num_masters_game: self
                .inner
                .property_int_value_cf(self.cf_masters_game, ESTIMATE_NUM_KEYS)?
                .unwrap_or(0),
        })
    }

    pub fn has_game(&self, id: GameId) -> Result<bool, rocksdb::Error> {
        self.inner
            .get_pinned_cf(self.cf_masters_game, id.to_bytes())
            .map(|maybe_entry| maybe_entry.is_some())
    }

    pub fn game(&self, id: GameId) -> Result<Option<MastersGame>, rocksdb::Error> {
        Ok(self
            .inner
            .get_pinned_cf(self.cf_masters_game, id.to_bytes())?
            .map(|buf| serde_json::from_slice(&buf).expect("deserialize masters game")))
    }

    pub fn games<I: IntoIterator<Item = GameId>>(
        &self,
        ids: I,
    ) -> Result<Vec<Option<MastersGame>>, rocksdb::Error> {
        let mut opt = ReadOptions::default();
        opt.set_ignore_range_deletions(true);
        self.inner
            .batched_multi_get_cf_opt(
                self.cf_masters_game,
                ids.into_iter().map(|id| id.to_bytes()),
                false,
                &opt,
            )
            .into_iter()
            .map(|maybe_buf_or_err| {
                maybe_buf_or_err.map(|maybe_buf| {
                    maybe_buf
                        .map(|buf| serde_json::from_slice(&buf).expect("deserialize masters game"))
                })
            })
            .collect()
    }

    pub fn has(&self, key: Key) -> Result<bool, rocksdb::Error> {
        self.inner
            .get_pinned_cf(self.cf_masters, key.into_bytes())
            .map(|maybe_entry| maybe_entry.is_some())
    }

    pub fn read(
        &self,
        key: KeyPrefix,
        since: Year,
        until: Year,
        cache_hint: CacheHint,
    ) -> Result<MastersEntry, rocksdb::Error> {
        let mut entry = MastersEntry::default();

        let mut opt = ReadOptions::default();
        opt.fill_cache(cache_hint.should_fill_cache());
        opt.set_ignore_range_deletions(true);
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(key.with_year(since).into_bytes());
        opt.set_iterate_upper_bound(key.with_year(until.add_years_saturating(1)).into_bytes());

        let mut iter = self.inner.raw_iterator_cf_opt(self.cf_masters, opt);
        iter.seek_to_first();

        while let Some(mut value) = iter.value() {
            entry.extend_from_reader(&mut value);
            iter.next();
        }

        iter.status().map(|_| entry)
    }

    pub fn batch(&self) -> MastersBatch<'_> {
        MastersBatch {
            db: self,
            batch: WriteBatch::default(),
        }
    }
}

pub struct MastersBatch<'a> {
    db: &'a MastersDatabase<'a>,
    batch: WriteBatch,
}

impl MastersBatch<'_> {
    pub fn merge(&mut self, key: Key, entry: MastersEntry) {
        let mut buf = Vec::with_capacity(MastersEntry::SIZE_HINT);
        entry.write(&mut buf);
        self.batch
            .merge_cf(self.db.cf_masters, key.into_bytes(), buf);
    }

    pub fn put_game(&mut self, id: GameId, game: &MastersGame) {
        self.batch.put_cf(
            self.db.cf_masters_game,
            id.to_bytes(),
            serde_json::to_vec(game).expect("serialize masters game"),
        );
    }

    pub fn commit(self) -> Result<(), rocksdb::Error> {
        self.db.inner.write(self.batch)
    }
}

pub struct LichessDatabase<'a> {
    inner: &'a DB,

    cf_lichess: &'a ColumnFamily,
    cf_lichess_game: &'a ColumnFamily,

    cf_player: &'a ColumnFamily,
    cf_player_status: &'a ColumnFamily,
}

pub struct LichessStats {
    pub num_lichess: u64,
    pub num_lichess_game: u64,
    pub num_player: u64,
    pub num_player_status: u64,
}

impl LichessDatabase<'_> {
    pub fn compact(&self) {
        log::info!("running manual compaction for lichess ...");
        compact_column(self.inner, self.cf_lichess);
        log::info!("running manual compaction for lichess_game ...");
        compact_column(self.inner, self.cf_lichess_game);
        log::info!("running manual compaction for player ...");
        compact_column(self.inner, self.cf_player);
        log::info!("running manual compaction for player_status ...");
        compact_column(self.inner, self.cf_player_status);
    }

    pub fn estimate_stats(&self) -> Result<LichessStats, rocksdb::Error> {
        Ok(LichessStats {
            num_lichess: self
                .inner
                .property_int_value_cf(self.cf_lichess, ESTIMATE_NUM_KEYS)?
                .unwrap_or(0),
            num_lichess_game: self
                .inner
                .property_int_value_cf(self.cf_lichess_game, ESTIMATE_NUM_KEYS)?
                .unwrap_or(0),
            num_player: self
                .inner
                .property_int_value_cf(self.cf_player, ESTIMATE_NUM_KEYS)?
                .unwrap_or(0),
            num_player_status: self
                .inner
                .property_int_value_cf(self.cf_player_status, ESTIMATE_NUM_KEYS)?
                .unwrap_or(0),
        })
    }

    pub fn game(&self, id: GameId) -> Result<Option<LichessGame>, rocksdb::Error> {
        Ok(self
            .inner
            .get_pinned_cf(self.cf_lichess_game, id.to_bytes())?
            .map(|buf| LichessGame::read(&mut buf.as_ref())))
    }

    pub fn games<I: IntoIterator<Item = GameId>>(
        &self,
        ids: I,
    ) -> Result<Vec<Option<LichessGame>>, rocksdb::Error> {
        let mut opt = ReadOptions::default();
        opt.set_ignore_range_deletions(true);
        self.inner
            .batched_multi_get_cf_opt(
                self.cf_lichess_game,
                ids.into_iter().map(|id| id.to_bytes()),
                false,
                &opt,
            )
            .into_iter()
            .map(|maybe_buf_or_err| {
                maybe_buf_or_err
                    .map(|maybe_buf| maybe_buf.map(|buf| LichessGame::read(&mut &buf[..])))
            })
            .collect()
    }

    pub fn read_lichess(
        &self,
        key: &KeyPrefix,
        filter: &LichessQueryFilter,
        limits: &Limits,
        history: HistoryWanted,
        cache_hint: CacheHint,
    ) -> Result<(PreparedResponse, Option<History>), rocksdb::Error> {
        let mut entry = LichessEntry::default();
        let mut history = match history {
            HistoryWanted::No => None,
            HistoryWanted::Yes => Some(HistoryBuilder::new_between(filter.since, filter.until)),
        };

        let mut opt = ReadOptions::default();
        opt.fill_cache(cache_hint.should_fill_cache());
        opt.set_ignore_range_deletions(true);
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(
            key.with_month(filter.since.unwrap_or_else(Month::min_value))
                .into_bytes(),
        );
        opt.set_iterate_upper_bound(
            key.with_month(
                filter
                    .until
                    .map_or(Month::max_value(), |m| m.add_months_saturating(1)),
            )
            .into_bytes(),
        );

        let mut iter = self.inner.raw_iterator_cf_opt(self.cf_lichess, opt);
        iter.seek_to_first();

        while let Some((key, mut value)) = iter.item() {
            entry.extend_from_reader(&mut value);

            if let Some(ref mut history) = history {
                history.record_difference(
                    Key::try_from(key)
                        .expect("lichess key size")
                        .month()
                        .expect("read lichess key suffix"),
                    entry.total(filter),
                );
            }

            iter.next();
        }

        iter.status().map(|_| {
            (
                entry.prepare(filter, limits),
                history.map(HistoryBuilder::build),
            )
        })
    }

    pub fn read_player(
        &self,
        key: &KeyPrefix,
        since: Month,
        until: Month,
        cache_hint: CacheHint,
    ) -> Result<PlayerEntry, rocksdb::Error> {
        let mut entry = PlayerEntry::default();

        let mut opt = ReadOptions::default();
        opt.fill_cache(cache_hint.should_fill_cache());
        opt.set_ignore_range_deletions(true);
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(key.with_month(since).into_bytes());
        opt.set_iterate_upper_bound(key.with_month(until.add_months_saturating(1)).into_bytes());

        let mut iter = self.inner.raw_iterator_cf_opt(self.cf_player, opt);
        iter.seek_to_first();

        while let Some(mut value) = iter.value() {
            entry.extend_from_reader(&mut value);
            iter.next();
        }

        iter.status().map(|_| entry)
    }

    pub fn player_status(&self, id: &UserId) -> Result<Option<PlayerStatus>, rocksdb::Error> {
        Ok(self
            .inner
            .get_pinned_cf(self.cf_player_status, id.as_lowercase_str())?
            .map(|buf| PlayerStatus::read(&mut buf.as_ref())))
    }

    pub fn put_player_status(
        &self,
        id: &UserId,
        status: &PlayerStatus,
    ) -> Result<(), rocksdb::Error> {
        let mut buf = Vec::with_capacity(PlayerStatus::SIZE_HINT);
        status.write(&mut buf);
        self.inner
            .put_cf(self.cf_player_status, id.as_lowercase_str(), buf)
    }

    pub fn batch(&self) -> LichessBatch<'_> {
        LichessBatch {
            inner: self,
            batch: WriteBatch::default(),
        }
    }
}

pub struct LichessBatch<'a> {
    inner: &'a LichessDatabase<'a>,
    batch: WriteBatch,
}

impl LichessBatch<'_> {
    pub fn merge_lichess(&mut self, key: Key, entry: LichessEntry) {
        let mut buf = Vec::with_capacity(LichessEntry::SIZE_HINT);
        entry.write(&mut buf);
        self.batch
            .merge_cf(self.inner.cf_lichess, key.into_bytes(), buf);
    }

    pub fn merge_game(&mut self, id: GameId, info: LichessGame) {
        let mut buf = Vec::with_capacity(LichessGame::SIZE_HINT);
        info.write(&mut buf);
        self.batch
            .merge_cf(self.inner.cf_lichess_game, id.to_bytes(), buf);
    }

    pub fn merge_player(&mut self, key: Key, entry: PlayerEntry) {
        let mut buf = Vec::with_capacity(PlayerEntry::SIZE_HINT);
        entry.write(&mut buf);
        self.batch
            .merge_cf(self.inner.cf_player, key.into_bytes(), buf);
    }

    pub fn commit(self) -> Result<(), rocksdb::Error> {
        self.inner.inner.write(self.batch)
    }
}

fn lichess_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    let mut entry = LichessEntry::default();
    for mut op in existing.into_iter().chain(operands.into_iter()) {
        entry.extend_from_reader(&mut op);
    }
    let mut buf = Vec::new();
    entry.write(&mut buf);
    Some(buf)
}

fn lichess_game_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    // Take latest game info, but merge index status.
    let mut info: Option<LichessGame> = None;
    let mut size_hint = 0;
    for mut op in existing.into_iter().chain(operands.into_iter()) {
        size_hint = op.len();
        let mut new_info = LichessGame::read(&mut op);
        if let Some(old_info) = info {
            new_info.indexed_player.white |= old_info.indexed_player.white;
            new_info.indexed_player.black |= old_info.indexed_player.black;
            new_info.indexed_lichess |= old_info.indexed_lichess;
        }
        info = Some(new_info);
    }
    info.map(|info| {
        let mut buf = Vec::with_capacity(size_hint);
        info.write(&mut buf);
        buf
    })
}

fn player_merge(_key: &[u8], existing: Option<&[u8]>, operands: &MergeOperands) -> Option<Vec<u8>> {
    let mut entry = PlayerEntry::default();
    for mut op in existing.into_iter().chain(operands.into_iter()) {
        entry.extend_from_reader(&mut op);
    }
    let mut buf = Vec::new();
    entry.write(&mut buf);
    Some(buf)
}

fn masters_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    let mut entry = MastersEntry::default();
    for mut op in existing.into_iter().chain(operands.into_iter()) {
        entry.extend_from_reader(&mut op);
    }
    let mut buf = Vec::new();
    entry.write(&mut buf);
    Some(buf)
}

fn compact_column(db: &DB, cf: &ColumnFamily) {
    db.compact_range_cf(cf, None::<&[u8]>, None::<&[u8]>);
}
