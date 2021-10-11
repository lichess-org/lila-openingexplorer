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
    model::PersonalKeyBuilder,
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
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

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

    let mut pos = Zobrist::new(VariantPosition::from_setup(
        variant,
        &Fen::from(query.fen),
        CastlingMode::Chess960,
    )?);

    let opening = openings.play_and_classify(&mut pos, query.play)?;

    let key = PersonalKeyBuilder::with_user_pov(&query.player.into(), query.color)
        .with_zobrist(variant, pos.zobrist_hash());
    let queryable = db.queryable();
    dbg!(queryable
        .db
        .get_cf(queryable.cf_personal, dbg!(key.prefix()))
        .expect("get cf personal"));
    Ok(Json(PersonalResponse { opening }))
}
