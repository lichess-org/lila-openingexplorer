pub mod api;
pub mod db;
pub mod indexer;
pub mod model;
pub mod opening;
pub mod util;

use crate::{
    api::{Error, GameRow, GameRowWithUci, PersonalMoveRow, PersonalQuery, PersonalResponse},
    db::Database,
    indexer::{IndexerOpt, IndexerStub},
    model::{AnnoLichess, PersonalKeyBuilder},
    opening::Openings,
};
use axum::{
    extract::{Extension, Path, Query},
    handler::get,
    http::StatusCode,
    response::Json,
    AddExtensionLayer, Router,
};
use clap::Clap;
use shakmaty::{fen::Fen, variant::VariantPosition, zobrist::Zobrist, CastlingMode};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

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
    env_logger::init();

    let opt = Opt::parse();

    let openings: &'static Openings = Box::leak(Box::new(Openings::new()));
    let db = Arc::new(Database::open(opt.db).expect("db"));
    let (indexer, join_handle) = IndexerStub::spawn(db.clone(), opt.indexer);

    let app = Router::new()
        .route("/personal", get(personal))
        .route("/admin/prop/:prop", get(prop))
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

async fn prop(
    Extension(db): Extension<Arc<Database>>,
    Path(prop): Path<String>,
) -> Result<String, StatusCode> {
    db.queryable()
        .property(&prop)
        .expect("get property")
        .ok_or(StatusCode::NOT_FOUND)
}

async fn personal(
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Extension(indexer): Extension<IndexerStub>,
    Query(query): Query<PersonalQuery>,
) -> Result<Json<PersonalResponse>, Error> {
    if query.update {
        let _status = indexer.index_player(query.player.clone()).await?;
    }

    let variant = query.variant.into();

    let mut pos = Zobrist::new(match query.fen {
        Some(fen) => VariantPosition::from_setup(variant, &Fen::from(fen), CastlingMode::Chess960)?,
        None => VariantPosition::new(variant),
    });

    let opening = openings.classify_and_play(&mut pos, query.play)?;

    let key = PersonalKeyBuilder::with_user_pov(&query.player.into(), query.color)
        .with_zobrist(variant, pos.zobrist_hash());

    let queryable = db.queryable();
    let filtered = queryable
        .get_personal(key, AnnoLichess::from_year(query.since))
        .expect("get personal")
        .prepare(pos.into_inner(), query.filter);

    Ok(Json(PersonalResponse {
        total: filtered.total,
        moves: filtered
            .moves
            .into_iter()
            .map(|row| PersonalMoveRow {
                uci: row.uci,
                san: row.san,
                stats: row.stats,
                game: row.game.and_then(|id| {
                    queryable
                        .get_game_info(id)
                        .expect("get game")
                        .map(|info| GameRow { id, info })
                }),
            })
            .collect(),
        recent_games: filtered
            .recent_games
            .into_iter()
            .flat_map(|(uci, id)| {
                queryable
                    .get_game_info(id)
                    .expect("get game")
                    .map(|info| GameRowWithUci {
                        uci,
                        row: GameRow { id, info },
                    })
            })
            .collect(),
        opening,
    }))
}
