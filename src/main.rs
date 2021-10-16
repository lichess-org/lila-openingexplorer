pub mod api;
pub mod db;
pub mod indexer;
pub mod model;
pub mod opening;
pub mod util;

use crate::{
    api::{
        Error, GameRow, GameRowWithUci, PersonalMoveRow, PersonalQuery, PersonalQueryFilter,
        PersonalResponse,
    },
    db::Database,
    indexer::{IndexerOpt, IndexerStub},
    model::{AnnoLichess, PersonalKeyBuilder, PersonalKeyPrefix},
    opening::{Opening, Openings},
    util::NdJson,
};
use axum::{
    extract::{Extension, Path, Query},
    handler::get,
    http::StatusCode,
    AddExtensionLayer, Router,
};
use clap::Clap;
use futures_util::stream::Stream;
use shakmaty::{fen::Fen, variant::VariantPosition, zobrist::Zobrist, CastlingMode};
use std::{mem, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::oneshot;

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
    env_logger::init_from_env(
        env_logger::Env::new()
            .filter("EXPLORER_LOG")
            .write_style("EXPLORER_LOG_STYLE"),
    );

    let opt = Opt::parse();

    let openings: &'static Openings = Box::leak(Box::new(Openings::build_table()));
    let db = Arc::new(Database::open(opt.db).expect("db"));
    let (indexer, join_handle) = IndexerStub::spawn(db.clone(), opt.indexer);

    let app = Router::new()
        .route("/admin/:prop", get(db_property))
        .route("/admin/game/:prop", get(game_property))
        .route("/admin/personal/:prop", get(personal_property))
        .route("/personal", get(personal))
        .route("/test", get(test))
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

async fn test() -> NdJson<impl futures_util::stream::Stream<Item = u32>> {
    NdJson::new(futures_util::stream::unfold(0, |state| async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        Some((state, state + 1))
    }))
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

struct PersonalStreamState {
    indexing: Option<oneshot::Receiver<()>>,
    key: PersonalKeyPrefix,
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
    let indexing = if query.update {
        Some(indexer.index_player(query.player.clone()).await)
    } else {
        None
    };

    let variant = query.variant.into();

    let mut pos = Zobrist::new(match query.fen {
        Some(fen) => VariantPosition::from_setup(variant, &Fen::from(fen), CastlingMode::Chess960)?,
        None => VariantPosition::new(variant),
    });

    let opening = openings.classify_and_play(&mut pos, query.play)?;

    let key = PersonalKeyBuilder::with_user_pov(&query.player.into(), query.color)
        .with_zobrist(variant, pos.zobrist_hash());

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

    Ok(NdJson::new(futures_util::stream::unfold(
        state,
        |mut state| async move {
            if state.done {
                return None;
            }

            let first = mem::replace(&mut state.first, false);
            state.done = match state.indexing {
                Some(ref mut indexing) => {
                    tokio::select! {
                        _ = indexing => true,
                        _ = tokio::time::sleep(Duration::from_millis(if first { 200 } else { 5000 })) => false,
                    }
                }
                None => true,
            };
            let queryable = state.db.queryable();
            let filtered = queryable
                .get_personal(&state.key, AnnoLichess::from_year(state.filter.since))
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
    )))
}
