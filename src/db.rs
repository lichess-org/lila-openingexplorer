use crate::{
    api::GameRow,
    model::{AnnoLichess, GameId, GameInfo, PersonalEntry, PersonalKey, PersonalKeyPrefix},
};
use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MergeOperands, Options};
use std::io::Cursor;
use std::path::Path;

#[derive(Debug)]
pub struct Database {
    inner: DBWithThreadMode<rocksdb::SingleThreaded>,
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Database, rocksdb::Error> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let mut personal_opts = Options::default();
        personal_opts.set_merge_operator_associative("personal merge", personal_merge);

        let inner = DBWithThreadMode::open_cf_descriptors(
            &db_opts,
            path,
            vec![
                ColumnFamilyDescriptor::new("personal", personal_opts),
                ColumnFamilyDescriptor::new("game", Options::default()),
            ],
        )?;

        Ok(Database { inner })
    }

    pub fn queryable(&self) -> QueryableDatabase<'_> {
        QueryableDatabase {
            db: &self.inner,
            cf_personal: self.inner.cf_handle("personal").expect("cf personal"),
            cf_game: self.inner.cf_handle("game").expect("cf game"),
        }
    }
}

pub struct QueryableDatabase<'a> {
    pub db: &'a DBWithThreadMode<rocksdb::SingleThreaded>,
    pub cf_personal: &'a rocksdb::ColumnFamily,
    pub cf_game: &'a rocksdb::ColumnFamily,
}

impl QueryableDatabase<'_> {
    pub fn put_game_info(&self, id: GameId, info: GameInfo) -> Result<(), rocksdb::Error> {
        let mut cursor = Cursor::new(Vec::new());
        info.write(&mut cursor).expect("serialize game info");
        self.db
            .put_cf(self.cf_game, id.to_bytes(), cursor.into_inner())
    }

    pub fn get_game_info(&self, id: GameId) -> Result<Option<GameRow>, rocksdb::Error> {
        Ok(self.db.get_cf(self.cf_game, id.to_bytes())?.map(|buf| {
            let mut cursor = Cursor::new(buf);
            GameRow {
                id,
                info: GameInfo::read(&mut cursor).expect("deserialize game info"),
            }
        }))
    }

    pub fn merge_personal(
        &self,
        key: PersonalKey,
        entry: PersonalEntry,
    ) -> Result<(), rocksdb::Error> {
        let mut cursor = Cursor::new(Vec::new());
        entry.write(&mut cursor).expect("serialize personal entry");
        self.db
            .merge_cf(self.cf_personal, key.into_bytes(), cursor.into_inner())
    }

    pub fn get_personal(
        &self,
        key: PersonalKeyPrefix,
        since: AnnoLichess,
    ) -> Result<PersonalEntry, rocksdb::Error> {
        let mut entry = PersonalEntry::default();

        let mut end = rocksdb::ReadOptions::default();
        end.set_iterate_upper_bound(key.with_year(AnnoLichess::MAX).into_bytes());
        let iterator = self.db.iterator_cf_opt(
            self.cf_personal,
            end,
            rocksdb::IteratorMode::From(
                &key.with_year(since).into_bytes(),
                rocksdb::Direction::Forward,
            ),
        );

        for (_key, value) in iterator {
            let mut cursor = Cursor::new(value);
            entry
                .extend_from_reader(&mut cursor)
                .expect("deserialize personal entry");
        }

        Ok(entry)
    }
}

fn personal_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &mut MergeOperands,
) -> Option<Vec<u8>> {
    let mut entry = PersonalEntry::default();
    let mut size_hint = 0;
    if let Some(existing) = existing {
        let mut cursor = Cursor::new(existing);
        entry
            .extend_from_reader(&mut cursor)
            .expect("read existing personal entry");
        size_hint += existing.len();
    }
    for op in operands {
        let mut cursor = Cursor::new(op);
        entry
            .extend_from_reader(&mut cursor)
            .expect("read personal merge operand");
        size_hint += op.len();
    }
    let mut cursor = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut cursor).expect("write personal entry");
    Some(cursor.into_inner())
}
