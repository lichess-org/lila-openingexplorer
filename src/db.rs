use std::path::Path;

use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamily, ColumnFamilyDescriptor, DBCompressionType,
    MergeOperands, Options, ReadOptions, SliceTransform, WriteBatch, DB,
};

use crate::{
    api::{ExplorerHistorySegment, LichessQueryFilter},
    model::{
        GameId, Key, KeyPrefix, LichessEntry, LichessGame, MastersEntry, MastersGame, Month,
        PlayerEntry, PlayerStatus, Stats, UserId, Year,
    },
};

#[derive(Debug)]
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
        table_opts.set_index_block_restart_interval(16); // Save index space
        table_opts.set_cache_index_and_filter_blocks(true);
        table_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
        table_opts.set_hybrid_ribbon_filter(8.0, 1);
        table_opts.set_whole_key_filtering(self.prefix.is_none()); // Only prefix seeks for positions
        table_opts.set_format_version(5);

        let mut cf_opts = Options::default();
        cf_opts.set_block_based_table_factory(&table_opts);
        cf_opts.set_compression_type(DBCompressionType::Lz4);
        cf_opts.set_bottommost_compression_type(DBCompressionType::Zstd);
        cf_opts.set_level_compaction_dynamic_level_bytes(false); // Infinitely growing database
        cf_opts.set_optimize_filters_for_hits(true); // 90% filter size reduction

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
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Database, rocksdb::Error> {
        // Note on usage in async contexts: All database operations are
        // blocking (https://github.com/facebook/rocksdb/issues/3254).
        // Calls could be run in a thread-pool to avoid blocking other
        // requests, but (as benchmarked) this doesn't do much, because all
        // other requests are doing the same kind of briefly-blocking i/o
        // anyway.

        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        db_opts.set_max_background_jobs(4);
        db_opts.set_bytes_per_sync(1024 * 1024);

        // Target memory usage is 16 GiB. Leave the majority for operating
        // system page cache.
        let cache = Cache::new_lru_cache(4 * 1024 * 1024 * 1024)?;

        let inner = DB::open_cf_descriptors(
            &db_opts,
            path,
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

        log::info!("database opened");

        Ok(Database { inner })
    }

    pub fn compact(&self) {
        self.lichess().compact();
        self.masters().compact();
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

impl MastersDatabase<'_> {
    pub fn compact(&self) {
        compact_column(self.inner, self.cf_masters);
        compact_column(self.inner, self.cf_masters_game);
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
        self.inner
            .batched_multi_get_cf(
                self.cf_masters_game,
                ids.into_iter().map(|id| id.to_bytes()),
                false,
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
    ) -> Result<MastersEntry, rocksdb::Error> {
        let mut entry = MastersEntry::default();

        let mut opt = ReadOptions::default();
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

impl LichessDatabase<'_> {
    pub fn compact(&self) {
        compact_column(self.inner, self.cf_lichess);
        compact_column(self.inner, self.cf_lichess_game);
        compact_column(self.inner, self.cf_player);
        compact_column(self.inner, self.cf_player_status);
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
        self.inner
            .batched_multi_get_cf(
                self.cf_lichess_game,
                ids.into_iter().map(|id| id.to_bytes()),
                false,
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
        since: Option<Month>,
        until: Option<Month>,
    ) -> Result<LichessEntry, rocksdb::Error> {
        let mut entry = LichessEntry::default();

        let mut opt = ReadOptions::default();
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(
            key.with_month(since.unwrap_or_else(Month::min_value))
                .into_bytes(),
        );
        opt.set_iterate_upper_bound(
            key.with_month(until.map_or(Month::max_value(), |m| m.add_months_saturating(1)))
                .into_bytes(),
        );

        let mut iter = self.inner.raw_iterator_cf_opt(self.cf_lichess, opt);
        iter.seek_to_first();

        while let Some(mut value) = iter.value() {
            entry.extend_from_reader(&mut value);
            iter.next();
        }

        iter.status().map(|_| entry)
    }

    pub fn read_lichess_history(
        &self,
        key: &KeyPrefix,
        filter: &LichessQueryFilter,
    ) -> Result<Vec<ExplorerHistorySegment>, rocksdb::Error> {
        let mut history = Vec::new();
        let mut last_month: Option<Month> = filter.since;

        let mut opt = ReadOptions::default();
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
            // Fill gap.
            let month = Key::try_from(key)
                .expect("lichess key size")
                .month()
                .expect("read lichess key suffix");
            if let Some(mut last_month) = last_month {
                while last_month < month {
                    history.push(ExplorerHistorySegment {
                        month: last_month,
                        stats: Stats::default(),
                    });
                    last_month = last_month.add_months_saturating(1);
                }
            }
            last_month = Some(month.add_months_saturating(1));

            // Add entry.
            let mut entry = LichessEntry::default();
            entry.extend_from_reader(&mut value);
            history.push(ExplorerHistorySegment {
                month,
                stats: entry.total(filter),
            });

            iter.next();
        }

        iter.status().map(|_| history)
    }

    pub fn read_player(
        &self,
        key: &KeyPrefix,
        since: Month,
        until: Month,
    ) -> Result<PlayerEntry, rocksdb::Error> {
        let mut entry = PlayerEntry::default();

        let mut opt = ReadOptions::default();
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
