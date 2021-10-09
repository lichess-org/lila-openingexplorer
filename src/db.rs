use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MergeOperands, Options};
use std::io::Cursor;
use std::path::Path;
use crate::model::PersonalEntry;

#[derive(Debug)]
pub struct Database {
    pub inner: DBWithThreadMode<rocksdb::SingleThreaded>,
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
            vec![ColumnFamilyDescriptor::new("personal", personal_opts)],
        )?;

        Ok(Database {
            inner,
        })
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
    let mut writer = Cursor::new(Vec::with_capacity(size_hint));
    entry.write(&mut writer).expect("write personal entry");
    Some(writer.into_inner())
}
