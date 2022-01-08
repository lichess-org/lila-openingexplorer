use std::{io::Cursor, path::Path};

use rocksdb::{
    merge_operator::MergeFn, BlockBasedIndexType, BlockBasedOptions, Cache, ColumnFamily,
    ColumnFamilyDescriptor, MergeOperands, Options, ReadOptions, SliceTransform, WriteBatch, DB,
};

use crate::model::{
    GameId, Key, KeyPrefix, LichessEntry, LichessGame, MastersEntry, MastersGame, Month,
    PlayerEntry, PlayerStatus, UserId, Year,
};

#[derive(Debug)]
pub struct Database {
    pub inner: DB,
}

fn column_family(
    name: &str,
    merge: Option<&str>,
    merge_fn: impl MergeFn + Clone,
    prefix: Option<usize>,
    block_size: usize,
    bloom_filter_bits: f64,
    cache: &Cache,
) -> ColumnFamilyDescriptor {
    let mut opts = Options::default();
    if let Some(merge) = merge {
        opts.set_merge_operator_associative(merge, merge_fn);
    }
    opts.set_prefix_extractor(match prefix {
        Some(prefix) => SliceTransform::create_fixed_prefix(prefix),
        None => SliceTransform::create_noop(),
    });
    let mut block_opts = BlockBasedOptions::default();
    block_opts.set_block_cache(cache);
    block_opts.set_index_type(BlockBasedIndexType::HashSearch);
    block_opts.set_block_size(block_size);
    if bloom_filter_bits > 0.0 {
        block_opts.set_hybrid_ribbon_filter(bloom_filter_bits, 1);
    }
    opts.set_block_based_table_factory(&block_opts);
    ColumnFamilyDescriptor::new(name, opts)
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Database, rocksdb::Error> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let cache = Cache::new_lru_cache(512 * 1024 * 1024)?;

        let inner = DB::open_cf_descriptors(
            &db_opts,
            path,
            vec![
                // Masters database
                column_family(
                    "masters",
                    Some("masters_merge"),
                    masters_merge,
                    Some(KeyPrefix::SIZE),
                    8 * 1024,
                    5.0,
                    &cache,
                ),
                column_family(
                    "masters_game",
                    None,
                    void_merge,
                    None,
                    4 * 1024,
                    0.0,
                    &cache,
                ),
                // Lichess database
                column_family(
                    "lichess",
                    Some("lichess_merge"),
                    lichess_merge,
                    Some(KeyPrefix::SIZE),
                    16 * 1024,
                    2.0,
                    &cache,
                ),
                column_family(
                    "lichess_game",
                    Some("lichess_game_merge"),
                    lichess_game_merge,
                    None,
                    4 * 1024,
                    0.0,
                    &cache,
                ),
                // Player database (also shares lichess_game)
                column_family(
                    "player",
                    Some("player_merge"),
                    player_merge,
                    Some(KeyPrefix::SIZE),
                    16 * 1024,
                    2.0,
                    &cache,
                ),
                column_family(
                    "player_status",
                    None,
                    void_merge,
                    None,
                    4 * 1024,
                    0.0,
                    &cache,
                ),
            ],
        )?;

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
            .multi_get_cf(
                ids.into_iter()
                    .map(|id| (self.cf_masters_game, id.to_bytes())),
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

        while let Some(value) = iter.value() {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize masters entry");
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
        let mut cursor = Cursor::new(Vec::with_capacity(MastersEntry::SIZE_HINT));
        entry.write(&mut cursor).expect("serialize masters entry");
        self.batch
            .merge_cf(self.db.cf_masters, key.into_bytes(), cursor.into_inner());
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
            .map(|buf| {
                let mut cursor = Cursor::new(buf);
                LichessGame::read(&mut cursor).expect("deserialize game info")
            }))
    }

    pub fn games<I: IntoIterator<Item = GameId>>(
        &self,
        ids: I,
    ) -> Result<Vec<Option<LichessGame>>, rocksdb::Error> {
        self.inner
            .multi_get_cf(
                ids.into_iter()
                    .map(|id| (self.cf_lichess_game, id.to_bytes())),
            )
            .into_iter()
            .map(|maybe_buf_or_err| {
                maybe_buf_or_err.map(|maybe_buf| {
                    maybe_buf.map(|buf| {
                        let mut cursor = Cursor::new(buf);
                        LichessGame::read(&mut cursor).expect("deserialize game info")
                    })
                })
            })
            .collect()
    }

    pub fn read_lichess(
        &self,
        key: &KeyPrefix,
        since: Month,
        until: Month,
    ) -> Result<LichessEntry, rocksdb::Error> {
        let mut entry = LichessEntry::default();

        let mut opt = ReadOptions::default();
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(key.with_month(since).into_bytes());
        opt.set_iterate_upper_bound(key.with_month(until.add_months_saturating(1)).into_bytes());

        let mut iter = self.inner.raw_iterator_cf_opt(self.cf_lichess, opt);
        iter.seek_to_first();

        while let Some(value) = iter.value() {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize lichess entry");
            iter.next();
        }

        iter.status().map(|_| entry)
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

        while let Some(value) = iter.value() {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize player entry");
            iter.next();
        }

        iter.status().map(|_| entry)
    }

    pub fn player_status(&self, id: &UserId) -> Result<Option<PlayerStatus>, rocksdb::Error> {
        Ok(self
            .inner
            .get_pinned_cf(self.cf_player_status, id.as_lowercase_str())?
            .map(|buf| {
                let mut cursor = Cursor::new(buf);
                PlayerStatus::read(&mut cursor).expect("deserialize status")
            }))
    }

    pub fn put_player_status(
        &self,
        id: &UserId,
        status: &PlayerStatus,
    ) -> Result<(), rocksdb::Error> {
        let mut cursor = Cursor::new(Vec::with_capacity(PlayerStatus::SIZE_HINT));
        status.write(&mut cursor).expect("serialize status");
        self.inner.put_cf(
            self.cf_player_status,
            id.as_lowercase_str(),
            cursor.into_inner(),
        )
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
        let mut cursor = Cursor::new(Vec::with_capacity(LichessEntry::SIZE_HINT));
        entry.write(&mut cursor).expect("serialize lichess entry");
        self.batch
            .merge_cf(self.inner.cf_lichess, key.into_bytes(), cursor.into_inner());
    }

    pub fn merge_game(&mut self, id: GameId, info: LichessGame) {
        let mut cursor = Cursor::new(Vec::with_capacity(LichessGame::SIZE_HINT));
        info.write(&mut cursor).expect("serialize game info");
        self.batch.merge_cf(
            self.inner.cf_lichess_game,
            id.to_bytes(),
            cursor.into_inner(),
        );
    }

    pub fn merge_player(&mut self, key: Key, entry: PlayerEntry) {
        let mut cursor = Cursor::new(Vec::with_capacity(PlayerEntry::SIZE_HINT));
        entry.write(&mut cursor).expect("serialize player entry");
        self.batch
            .merge_cf(self.inner.cf_player, key.into_bytes(), cursor.into_inner());
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
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        entry
            .extend_from_reader(&mut cursor)
            .expect("deserialize for lichess merge");
        size_hint += op.len();
    }
    let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut cursor).expect("write lichess entry");
    Some(cursor.into_inner())
}

fn lichess_game_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    // Take latest game info, but merge index status.
    let mut info: Option<LichessGame> = None;
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        let mut new_info = LichessGame::read(&mut cursor).expect("read for lichess game merge");
        if let Some(old_info) = info {
            new_info.indexed_player.white |= old_info.indexed_player.white;
            new_info.indexed_player.black |= old_info.indexed_player.black;
            new_info.indexed_lichess |= old_info.indexed_lichess;
        }
        info = Some(new_info);
        size_hint = op.len();
    }
    info.map(|info| {
        let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
        info.write(&mut cursor).expect("write lichess game");
        cursor.into_inner()
    })
}

fn player_merge(_key: &[u8], existing: Option<&[u8]>, operands: &MergeOperands) -> Option<Vec<u8>> {
    let mut entry = PlayerEntry::default();
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        entry
            .extend_from_reader(&mut cursor)
            .expect("deserialize for player merge");
        size_hint += op.len();
    }
    let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut cursor).expect("write player entry");
    Some(cursor.into_inner())
}

fn masters_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    let mut entry = MastersEntry::default();
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        entry
            .extend_from_reader(&mut cursor)
            .expect("deserialize for masters merge");
        size_hint += op.len();
    }
    let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut cursor).expect("write masters entry");
    Some(cursor.into_inner())
}

fn void_merge(_key: &[u8], _existing: Option<&[u8]>, _operands: &MergeOperands) -> Option<Vec<u8>> {
    unreachable!("void merge")
}

fn compact_column(db: &DB, cf: &ColumnFamily) {
    db.compact_range_cf(cf, None::<&[u8]>, None::<&[u8]>);
}
