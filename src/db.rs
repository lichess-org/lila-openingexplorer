use std::{io::Cursor, path::Path};

use rocksdb::{
    merge_operator::MergeFn, BlockBasedIndexType, BlockBasedOptions, ColumnFamily,
    ColumnFamilyDescriptor, DBWithThreadMode, IteratorMode, MergeOperands, Options, ReadOptions,
    SliceTransform, WriteBatch, DB,
};

use crate::model::{
    GameId, GameInfo, Key, KeyPrefix, LichessEntry, MasterEntry, MasterGame, Month, PersonalEntry,
    PersonalStatus, UserId, Year,
};

#[derive(Debug)]
pub struct Database {
    inner: DB,
}

fn column_family(
    name: &str,
    merge: Option<&str>,
    merge_fn: impl MergeFn + Clone,
    prefix: Option<usize>,
    block_size: usize,
    bloom_filter: i32,
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
    block_opts.set_index_type(BlockBasedIndexType::HashSearch);
    block_opts.set_block_size(block_size);
    if bloom_filter > 0 {
        block_opts.set_bloom_filter(bloom_filter, true);
    }
    opts.set_block_based_table_factory(&block_opts);
    ColumnFamilyDescriptor::new(name, opts)
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Database, rocksdb::Error> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let inner = DBWithThreadMode::open_cf_descriptors(
            &db_opts,
            path,
            vec![
                column_family(
                    "lichess",
                    Some("lichess merge"),
                    lichess_merge,
                    Some(KeyPrefix::SIZE),
                    4 * 1024,
                    5,
                ),
                column_family(
                    "personal",
                    Some("personal merge"),
                    personal_merge,
                    Some(KeyPrefix::SIZE),
                    4 * 1024,
                    6,
                ),
                column_family("game", Some("game merge"), game_merge, None, 1024, 0),
                column_family("player", None, void_merge, None, 1024, 0),
                column_family(
                    "master",
                    Some("master merge"),
                    master_merge,
                    Some(KeyPrefix::SIZE),
                    4 * 1024,
                    6,
                ),
                column_family("master_game", None, void_merge, None, 4 * 1024, 0),
            ],
        )?;

        Ok(Database { inner })
    }

    pub fn queryable(&self) -> QueryableDatabase<'_> {
        QueryableDatabase {
            db: &self.inner,
            cf_lichess: self.inner.cf_handle("lichess").expect("cf lichess"),
            cf_personal: self.inner.cf_handle("personal").expect("cf personal"),
            cf_game: self.inner.cf_handle("game").expect("cf game"),
            cf_player: self.inner.cf_handle("player").expect("cf player"),
            cf_master: self.inner.cf_handle("master").expect("cf master"),
            cf_master_game: self.inner.cf_handle("master_game").expect("cf master_game"),
        }
    }
}

pub struct QueryableDatabase<'a> {
    db: &'a DB,
    cf_lichess: &'a ColumnFamily,
    cf_personal: &'a ColumnFamily,
    cf_game: &'a ColumnFamily,
    cf_player: &'a ColumnFamily,
    cf_master: &'a ColumnFamily,
    cf_master_game: &'a ColumnFamily,
}

impl QueryableDatabase<'_> {
    pub fn db_property(&self, name: &str) -> Result<Option<String>, rocksdb::Error> {
        self.db.property_value(name)
    }

    pub fn game_property(&self, name: &str) -> Result<Option<String>, rocksdb::Error> {
        self.db.property_value_cf(self.cf_game, name)
    }

    pub fn personal_property(&self, name: &str) -> Result<Option<String>, rocksdb::Error> {
        self.db.property_value_cf(self.cf_personal, name)
    }

    pub fn get_game_info(&self, id: GameId) -> Result<Option<GameInfo>, rocksdb::Error> {
        Ok(self.db.get_cf(self.cf_game, id.to_bytes())?.map(|buf| {
            let mut cursor = Cursor::new(buf);
            GameInfo::read(&mut cursor).expect("deserialize game info")
        }))
    }

    pub fn get_lichess(
        &self,
        key: &KeyPrefix,
        since: Month,
        until: Month,
    ) -> Result<LichessEntry, rocksdb::Error> {
        let mut opt = ReadOptions::default();
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(key.with_month(since).into_bytes());
        opt.set_iterate_upper_bound(key.with_month(until.add_months_saturating(1)).into_bytes());

        let iterator = self
            .db
            .iterator_cf_opt(self.cf_lichess, opt, IteratorMode::Start);

        let mut entry = LichessEntry::default();
        for (_key, value) in iterator {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize lichess entry");
        }

        Ok(entry)
    }

    pub fn get_personal(
        &self,
        key: &KeyPrefix,
        since: Month,
        until: Month,
    ) -> Result<PersonalEntry, rocksdb::Error> {
        let mut opt = ReadOptions::default();
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(key.with_month(since).into_bytes());
        opt.set_iterate_upper_bound(key.with_month(until.add_months_saturating(1)).into_bytes());

        let iterator = self
            .db
            .iterator_cf_opt(self.cf_personal, opt, IteratorMode::Start);

        let mut entry = PersonalEntry::default();
        for (_key, value) in iterator {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize personal entry");
        }

        Ok(entry)
    }

    pub fn get_player_status(&self, id: &UserId) -> Result<Option<PersonalStatus>, rocksdb::Error> {
        Ok(self.db.get_cf(self.cf_player, id.as_str())?.map(|buf| {
            let mut cursor = Cursor::new(buf);
            PersonalStatus::read(&mut cursor).expect("deserialize status")
        }))
    }

    pub fn put_player_status(
        &self,
        id: &UserId,
        status: &PersonalStatus,
    ) -> Result<(), rocksdb::Error> {
        let mut cursor = Cursor::new(Vec::with_capacity(PersonalStatus::SIZE_HINT));
        status.write(&mut cursor).expect("serialize status");
        self.db
            .put_cf(self.cf_player, id.as_str(), cursor.into_inner())
    }

    pub fn has_master_game(&self, id: GameId) -> Result<bool, rocksdb::Error> {
        self.db
            .get_cf(self.cf_master_game, id.to_bytes())
            .map(|maybe_entry| maybe_entry.is_some())
    }

    pub fn get_master_game(&self, id: GameId) -> Result<Option<MasterGame>, rocksdb::Error> {
        Ok(self
            .db
            .get_cf(self.cf_master_game, id.to_bytes())?
            .map(|buf| serde_json::from_slice(&buf).expect("deserialize master game")))
    }

    pub fn has_master(&self, key: Key) -> Result<bool, rocksdb::Error> {
        self.db
            .get_cf(self.cf_master, key.into_bytes())
            .map(|maybe_entry| maybe_entry.is_some())
    }

    pub fn get_master(
        &self,
        key: KeyPrefix,
        since: Year,
        until: Year,
    ) -> Result<MasterEntry, rocksdb::Error> {
        let mut opt = ReadOptions::default();
        opt.set_prefix_same_as_start(true);
        opt.set_iterate_lower_bound(key.with_year(since).into_bytes());
        opt.set_iterate_upper_bound(key.with_year(until.add_years_saturating(1)).into_bytes());

        let iterator = self
            .db
            .iterator_cf_opt(self.cf_master, opt, IteratorMode::Start);

        let mut entry = MasterEntry::default();
        for (_key, value) in iterator {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize master entry");
        }

        Ok(entry)
    }

    pub fn batch(&self) -> Batch<'_> {
        Batch {
            queryable: self,
            batch: WriteBatch::default(),
        }
    }
}

pub struct Batch<'a> {
    queryable: &'a QueryableDatabase<'a>,
    batch: WriteBatch,
}

impl Batch<'_> {
    pub fn merge_lichess(&mut self, key: Key, entry: LichessEntry) {
        let mut cursor = Cursor::new(Vec::with_capacity(LichessEntry::SIZE_HINT));
        entry.write(&mut cursor).expect("serialize lichess entry");
        self.batch.merge_cf(
            self.queryable.cf_lichess,
            key.into_bytes(),
            cursor.into_inner(),
        );
    }

    pub fn merge_personal(&mut self, key: Key, entry: PersonalEntry) {
        let mut cursor = Cursor::new(Vec::with_capacity(PersonalEntry::SIZE_HINT));
        entry.write(&mut cursor).expect("serialize personal entry");
        self.batch.merge_cf(
            self.queryable.cf_personal,
            key.into_bytes(),
            cursor.into_inner(),
        );
    }

    pub fn merge_game_info(&mut self, id: GameId, info: GameInfo) {
        let mut cursor = Cursor::new(Vec::with_capacity(GameInfo::SIZE_HINT));
        info.write(&mut cursor).expect("serialize game info");
        self.batch
            .merge_cf(self.queryable.cf_game, id.to_bytes(), cursor.into_inner());
    }

    pub fn merge_master(&mut self, key: Key, entry: MasterEntry) {
        let mut cursor = Cursor::new(Vec::with_capacity(MasterEntry::SIZE_HINT));
        entry.write(&mut cursor).expect("serialize master entry");
        self.batch.merge_cf(
            self.queryable.cf_master,
            key.into_bytes(),
            cursor.into_inner(),
        );
    }

    pub fn put_master_game(&mut self, id: GameId, game: &MasterGame) {
        self.batch.put_cf(
            self.queryable.cf_master_game,
            id.to_bytes(),
            serde_json::to_vec(game).expect("serialize master game"),
        );
    }

    pub fn write(self) -> Result<(), rocksdb::Error> {
        self.queryable.db.write(self.batch)
    }
}

fn lichess_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &mut MergeOperands,
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

fn game_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &mut MergeOperands,
) -> Option<Vec<u8>> {
    // Take latest game info, but merge index status.
    let mut info: Option<GameInfo> = None;
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        let mut new_info = GameInfo::read(&mut cursor).expect("read for game merge");
        if let Some(old_info) = info {
            new_info.indexed_personal.white |= old_info.indexed_personal.white;
            new_info.indexed_personal.black |= old_info.indexed_personal.black;
            new_info.indexed_lichess |= old_info.indexed_lichess;
        }
        info = Some(new_info);
        size_hint = op.len();
    }
    info.map(|info| {
        let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
        info.write(&mut cursor).expect("write game");
        cursor.into_inner()
    })
}

fn personal_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &mut MergeOperands,
) -> Option<Vec<u8>> {
    let mut entry = PersonalEntry::default();
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        entry
            .extend_from_reader(&mut cursor)
            .expect("deserialize for personal merge");
        size_hint += op.len();
    }
    let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut cursor).expect("write personal entry");
    Some(cursor.into_inner())
}

fn master_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &mut MergeOperands,
) -> Option<Vec<u8>> {
    let mut entry = MasterEntry::default();
    let mut size_hint = 0;
    for op in existing.into_iter().chain(operands.into_iter()) {
        let mut cursor = Cursor::new(op);
        entry
            .extend_from_reader(&mut cursor)
            .expect("deserialize for master merge");
        size_hint += op.len();
    }
    let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut cursor).expect("write master entry");
    Some(cursor.into_inner())
}

fn void_merge(
    _key: &[u8],
    _existing: Option<&[u8]>,
    _operands: &mut MergeOperands,
) -> Option<Vec<u8>> {
    unreachable!("void merge operator only used to satisfy type checker")
}
