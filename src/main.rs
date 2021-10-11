pub mod api;
pub mod db;
pub mod indexer;
pub mod model;
pub mod opening;
pub mod util;

use crate::{
    api::{Error, PersonalQuery, PersonalResponse},
    db::Database,
    indexer::{IndexerOpt, IndexerStub},
    model::{AnnoLichess, PersonalEntry, PersonalKeyBuilder},
    opening::Openings,
};
use axum::{
    extract::{Extension, Query},
    handler::get,
    response::Json,
    AddExtensionLayer, Router,
};
use clap::Clap;
use shakmaty::{fen::Fen, variant::VariantPosition, zobrist::Zobrist, CastlingMode};
use std::{io::Cursor, net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Clap)]
struct Opt {
    #[clap(long = "bind", default_value = "127.0.0.1:9000")]
    bind: SocketAddr,
    #[clap(long = "db", default_value = "_db")]
    db: PathBuf,
    #[clap(flatten)]
    indexer: IndexerOpt,
}

#[tokio::main]
async fn main() {
    let opt = Opt::parse();

    let openings: &'static Openings = Box::leak(Box::new(Openings::new()));
    let db = Arc::new(Database::open(opt.db).expect("db"));
    let (indexer, join_handle) = IndexerStub::spawn(db.clone(), opt.indexer);

    let app = Router::new()
        .route("/personal", get(personal))
        .layer(AddExtensionLayer::new(openings))
        .layer(AddExtensionLayer::new(db))
        .layer(AddExtensionLayer::new(indexer));

    axum::Server::bind(&opt.bind)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("wait for ctrl-c");
        })
        .await
        .expect("bind");

    join_handle.await.expect("indexer");
}

async fn personal(
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Extension(indexer): Extension<IndexerStub>,
    Query(query): Query<PersonalQuery>,
) -> Result<Json<PersonalResponse>, Error> {
    if dbg!(&query).update {
        let _status = indexer.index_player(query.player.clone()).await?;
    }

    let variant = query.variant.into();

    let mut pos = Zobrist::new(match query.fen {
        Some(fen) => VariantPosition::from_setup(variant, &Fen::from(fen), CastlingMode::Chess960)?,
        None => VariantPosition::new(variant),
    });

    let opening = openings.play_and_classify(&mut pos, query.play)?;

    let key = PersonalKeyBuilder::with_user_pov(&query.player.into(), query.color)
        .with_zobrist(variant, pos.zobrist_hash());

    let mut entry = PersonalEntry::default();
    let queryable = db.queryable();
    let mut end = rocksdb::ReadOptions::default();
    end.set_iterate_upper_bound(key.with_year(AnnoLichess::MAX));
    let iterator = queryable.db.iterator_cf_opt(
        queryable.cf_personal,
        end,
        rocksdb::IteratorMode::From(
            &key.with_year(AnnoLichess::from_year(query.since)),
            rocksdb::Direction::Forward,
        ),
    );
    for (_key, value) in iterator {
        let mut cursor = Cursor::new(value);
        entry
            .extend_from_reader(&mut cursor)
            .expect("deserialize personal entry");
    }

    Ok(Json(query.filter.respond(pos.into_inner(), entry, opening)))
}
