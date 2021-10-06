pub mod api;
pub mod lila;
pub mod model;

use shakmaty::Color;
use futures_util::stream::StreamExt as _;

use self::model::PersonalEntry;
use rocksdb::{ColumnFamilyDescriptor, MergeOperands, Options, DB};
use std::io::Cursor;

struct _HashKey {
    pos: (),
    player: String,
    color: Color,
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

#[tokio::main]
async fn main() {
    let mut db_opts = Options::default();
    db_opts.create_if_missing(true);
    db_opts.create_missing_column_families(true);

    let mut personal_opts = Options::default();
    personal_opts.set_merge_operator_associative("personal merge", personal_merge);

    let db = DB::open_cf_descriptors(
        &db_opts,
        "_db",
        vec![ColumnFamilyDescriptor::new("personal", personal_opts)],
    )
    .expect("open db");

    let personal_column = db.cf_handle("personal").expect("personal cf");

    db.merge_cf(&personal_column, "k", "").expect("merge");

    let api = lila::Api::new();

    let mut games = api.user_games("revoof").await.expect("user games request");
    while let Some(game) = games.next().await {
        dbg!(game.expect("next game"));
    }
}
