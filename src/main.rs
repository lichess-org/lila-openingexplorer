#![forbid(unsafe_code)]

pub mod api;
pub mod db;
pub mod importer;
pub mod indexer;
pub mod model;
pub mod opening;
pub mod util;

use std::{mem, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    extract::{Extension, Path, Query},
    handler::{get, put},
    http::StatusCode,
    AddExtensionLayer, Json, Router,
};
use clap::Clap;
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use shakmaty::{fen::Fen, variant::VariantPosition, zobrist::Zobrist, CastlingMode};
use tokio::sync::watch;

use crate::{
    api::{
        Error, GameRow, GameRowWithUci, NdJson, PersonalMoveRow, PersonalQuery,
        PersonalQueryFilter, PersonalResponse,
    },
    db::Database,
    importer::MasterImporter,
    indexer::{IndexerOpt, IndexerStub},
    model::{GameId, KeyBuilder, KeyPrefix, MasterGameWithId, UserId},
    opening::{Opening, Openings},
    util::DedupStreamExt as _,
};
use crate::model::MasterGame;

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
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .filter("EXPLORER_LOG")
            .write_style("EXPLORER_LOG_STYLE"),
    )
    .format_timestamp(None)
    .format_module_path(false)
    .format_target(false)
    .init();

    let opt = Opt::parse();

    let openings: &'static Openings = Box::leak(Box::new(Openings::build_table()));
    let db = Arc::new(Database::open(opt.db).expect("db"));
    let (indexer, join_handles) = IndexerStub::spawn(Arc::clone(&db), opt.indexer);
    let master_importer = MasterImporter::new(Arc::clone(&db));

    let app = Router::new()
        .route("/admin/:prop", get(db_property))
        .route("/admin/game/:prop", get(game_property))
        .route("/admin/personal/:prop", get(personal_property))
        .route("/admin/explorer.indexing", get(num_indexing))
        .route("/import/master", put(master_import))
        .route("/master/pgn/:id", get(master_pgn))
        .route("/personal", get(personal))
        .layer(AddExtensionLayer::new(openings))
        .layer(AddExtensionLayer::new(db))
        .layer(AddExtensionLayer::new(indexer))
        .layer(AddExtensionLayer::new(master_importer));

    axum::Server::bind(&opt.bind)
        .serve(app.into_make_service())
        .await
        .expect("bind");

    for join_handle in join_handles {
        join_handle.await.expect("indexer");
    }
}

async fn db_property(
    Extension(db): Extension<Arc<Database>>,
    Path(prop): Path<String>,
) -> Result<String, StatusCode> {
    db.queryable()
        .db_property(&prop)
        .expect("get property")
        .ok_or(StatusCode::NOT_FOUND)
}

async fn game_property(
    Extension(db): Extension<Arc<Database>>,
    Path(prop): Path<String>,
) -> Result<String, StatusCode> {
    db.queryable()
        .game_property(&prop)
        .expect("get property")
        .ok_or(StatusCode::NOT_FOUND)
}

async fn personal_property(
    Extension(db): Extension<Arc<Database>>,
    Path(prop): Path<String>,
) -> Result<String, StatusCode> {
    db.queryable()
        .personal_property(&prop)
        .expect("get property")
        .ok_or(StatusCode::NOT_FOUND)
}

async fn num_indexing(Extension(indexer): Extension<IndexerStub>) -> String {
    indexer.num_indexing().await.to_string()
}

struct PersonalStreamState {
    indexing: Option<watch::Receiver<()>>,
    key: KeyPrefix,
    db: Arc<Database>,
    filter: PersonalQueryFilter,
    pos: VariantPosition,
    opening: Option<&'static Opening>,
    first: bool,
    done: bool,
}

async fn personal(
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Extension(indexer): Extension<IndexerStub>,
    Query(query): Query<PersonalQuery>,
) -> Result<NdJson<impl Stream<Item = PersonalResponse>>, Error> {
    let player = UserId::from(query.player);
    let indexing = indexer.index_player(&player).await;

    let variant = query.variant.into();

    let mut pos = Zobrist::new(match query.fen {
        Some(fen) => VariantPosition::from_setup(variant, &Fen::from(fen), CastlingMode::Chess960)?,
        None => VariantPosition::new(variant),
    });

    let opening = openings.classify_and_play(&mut pos, query.play)?;

    let key = KeyBuilder::personal(&player, query.color).with_zobrist(variant, pos.zobrist_hash());

    let state = PersonalStreamState {
        filter: query.filter,
        db,
        indexing,
        opening,
        key,
        pos: pos.into_inner(),
        first: true,
        done: false,
    };

    Ok(NdJson(futures_util::stream::unfold(
        state,
        |mut state| async move {
            if state.done {
                return None;
            }

            let first = mem::replace(&mut state.first, false);
            state.done = match state.indexing {
                Some(ref mut indexing) => {
                    tokio::select! {
                        _ = indexing.changed() => true,
                        _ = tokio::time::sleep(Duration::from_millis(if first { 0 } else { 1000 })) => false,
                    }
                }
                None => true,
            };

            let queryable = state.db.queryable();
            let filtered = queryable
                .get_personal(&state.key, state.filter.since, state.filter.until)
                .expect("get personal")
                .prepare(&state.pos, &state.filter);

            Some((
                PersonalResponse {
                    total: filtered.total,
                    moves: filtered
                        .moves
                        .into_iter()
                        .map(|row| PersonalMoveRow {
                            uci: row.uci,
                            san: row.san,
                            average_opponent_rating: row.stats.average_rating(),
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
                            queryable.get_game_info(id).expect("get game").map(|info| {
                                GameRowWithUci {
                                    uci,
                                    row: GameRow { id, info },
                                }
                            })
                        })
                        .collect(),
                    opening: state.opening,
                },
                state,
            ))
        },
    ).dedup_by_key(|res| res.total.total())))
}

async fn master_import(
    Json(body): Json<MasterGameWithId>,
    Extension(importer): Extension<MasterImporter>,
) -> Result<(), Error> {
    importer.import(body).await
}

#[serde_as]
#[derive(Deserialize)]
struct MasterGameId(#[serde_as(as = "DisplayFromStr")] GameId);

async fn master_pgn(
    Path(MasterGameId(id)): Path<MasterGameId>,
    Extension(db): Extension<Arc<Database>>,
) -> Result<MasterGame, StatusCode> {
    match db.queryable().get_master_game(id).expect("get master game") {
        Some(game) => Ok(game),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn master(
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Query(query): Query<MasterQuery>,
) -> Result<Json<()>, Error> {
}
