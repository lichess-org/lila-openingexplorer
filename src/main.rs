#![forbid(unsafe_code)]

pub mod api;
pub mod db;
pub mod importer;
pub mod indexer;
pub mod model;
pub mod opening;
pub mod util;

use std::{mem, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use clap::Parser;
use futures_util::stream::Stream;
use moka::future::Cache;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use shakmaty::{
    san::{San, SanPlus},
    uci::Uci,
    variant::VariantPosition,
    zobrist::ZobristHash,
    Color, EnPassantMode,
};
use tikv_jemallocator::Jemalloc;
use tokio::{sync::watch, task};

use crate::{
    api::{
        Error, ExplorerGame, ExplorerGameWithUci, ExplorerHistoryResponse, ExplorerMove,
        ExplorerResponse, LichessHistoryQuery, LichessQuery, Limits, MastersQuery, NdJson,
        PlayPosition, PlayerQuery, PlayerQueryFilter,
    },
    db::{Database, DbOpt, LichessDatabase},
    importer::{LichessGameImport, LichessImporter, MastersImporter},
    indexer::{IndexerOpt, IndexerStub},
    model::{GameId, KeyBuilder, KeyPrefix, MastersGame, MastersGameWithId, PreparedMove, UserId},
    opening::{Opening, Openings},
    util::DedupStreamExt as _,
};

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Parser)]
struct Opt {
    /// Binding address. Note that administrative endpoints must be protected
    /// using a reverse proxy.
    #[arg(long, default_value = "127.0.0.1:9002")]
    bind: SocketAddr,
    /// Allow access from all origins.
    #[arg(long)]
    cors: bool,
    /// Number of cached responses for masters and Lichess database each.
    #[arg(long, default_value = "10000")]
    cached_responses: u64,
    #[command(flatten)]
    db: DbOpt,
    #[command(flatten)]
    indexer: IndexerOpt,
}

type ExplorerCache<T> = Cache<T, Result<Json<ExplorerResponse>, Error>>;

#[derive(Clone)]
struct AppState {
    openings: &'static Openings,
    db: Arc<Database>,
    lichess_cache: ExplorerCache<LichessQuery>,
    masters_cache: ExplorerCache<MastersQuery>,
    lichess_importer: LichessImporter,
    masters_importer: MastersImporter,
    indexer: IndexerStub,
}

impl FromRef<AppState> for &'static Openings {
    fn from_ref(state: &AppState) -> &'static Openings {
        state.openings
    }
}

impl FromRef<AppState> for Arc<Database> {
    fn from_ref(state: &AppState) -> Arc<Database> {
        Arc::clone(&state.db)
    }
}

impl FromRef<AppState> for ExplorerCache<LichessQuery> {
    fn from_ref(state: &AppState) -> ExplorerCache<LichessQuery> {
        state.lichess_cache.clone()
    }
}

impl FromRef<AppState> for ExplorerCache<MastersQuery> {
    fn from_ref(state: &AppState) -> ExplorerCache<MastersQuery> {
        state.masters_cache.clone()
    }
}

impl FromRef<AppState> for LichessImporter {
    fn from_ref(state: &AppState) -> LichessImporter {
        state.lichess_importer.clone()
    }
}

impl FromRef<AppState> for MastersImporter {
    fn from_ref(state: &AppState) -> MastersImporter {
        state.masters_importer.clone()
    }
}

impl FromRef<AppState> for IndexerStub {
    fn from_ref(state: &AppState) -> IndexerStub {
        state.indexer.clone()
    }
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

    let db = Arc::new(Database::open(opt.db).expect("db"));
    let (indexer, join_handles) = IndexerStub::spawn(Arc::clone(&db), opt.indexer);

    let app = Router::new()
        .route("/monitor/cf/:cf/:prop", get(cf_prop))
        .route("/monitor/db/:prop", get(db_prop))
        .route("/monitor/indexing", get(num_indexing))
        .route("/compact", post(compact))
        .route("/import/masters", put(masters_import))
        .route("/import/lichess", put(lichess_import))
        .route("/masters/pgn/:id", get(masters_pgn))
        .route("/masters", get(masters))
        .route("/lichess", get(lichess))
        .route("/lichess/history", get(lichess_history))
        .route("/player", get(player))
        .route("/master/pgn/:id", get(masters_pgn)) // bc
        .route("/master", get(masters)) // bc
        .route("/personal", get(player)) // bc
        .with_state(AppState {
            openings: Box::leak(Box::new(Openings::build_table())),
            lichess_cache: Cache::builder()
                .max_capacity(opt.cached_responses)
                .time_to_live(Duration::from_secs(60 * 60))
                .build(),
            masters_cache: Cache::builder()
                .max_capacity(opt.cached_responses)
                .time_to_live(Duration::from_secs(60 * 60))
                .build(),
            lichess_importer: LichessImporter::new(Arc::clone(&db)),
            masters_importer: MastersImporter::new(Arc::clone(&db)),
            indexer,
            db,
        });

    let app = if opt.cors {
        app.layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
            axum::http::HeaderValue::from_static("*"),
        ))
    } else {
        app
    };

    axum::Server::bind(&opt.bind)
        .serve(app.into_make_service())
        .await
        .expect("bind");

    for join_handle in join_handles {
        join_handle.await.expect("indexer");
    }
}

#[derive(Deserialize)]
struct ColumnFamilyProp {
    cf: String,
    prop: String,
}

async fn cf_prop(
    Path(path): Path<ColumnFamilyProp>,
    State(db): State<Arc<Database>>,
) -> Result<String, StatusCode> {
    task::spawn_blocking(move || {
        db.inner
            .cf_handle(&path.cf)
            .and_then(|cf| {
                db.inner
                    .property_value_cf(cf, &path.prop)
                    .expect("property value")
            })
            .ok_or(StatusCode::NOT_FOUND)
    })
    .await
    .expect("blocking cf prop")
}

async fn db_prop(
    Path(prop): Path<String>,
    State(db): State<Arc<Database>>,
) -> Result<String, StatusCode> {
    task::spawn_blocking(move || {
        db.inner
            .property_value(&prop)
            .expect("property value")
            .ok_or(StatusCode::NOT_FOUND)
    })
    .await
    .expect("blocking db prop")
}

async fn num_indexing(State(indexer): State<IndexerStub>) -> String {
    indexer.num_indexing().await.to_string()
}

async fn compact(State(db): State<Arc<Database>>) {
    task::spawn_blocking(move || db.compact())
        .await
        .expect("blocking compact");
}

fn finalize_lichess_moves(
    moves: Vec<PreparedMove>,
    pos: &VariantPosition,
    lichess_db: &LichessDatabase,
) -> Vec<ExplorerMove> {
    moves
        .into_iter()
        .map(|p| ExplorerMove {
            stats: p.stats,
            san: p.uci.to_move(pos).map_or(
                SanPlus {
                    san: San::Null,
                    suffix: None,
                },
                |m| SanPlus::from_move(pos.clone(), &m),
            ),
            uci: p.uci,
            average_rating: p.average_rating,
            average_opponent_rating: p.average_opponent_rating,
            performance: p.performance,
            game: p.game.and_then(|id| {
                lichess_db
                    .game(id)
                    .expect("get game")
                    .map(|info| ExplorerGame::from_lichess(id, info))
            }),
        })
        .collect()
}

fn finalize_lichess_games(
    games: Vec<(Uci, GameId)>,
    lichess_db: &LichessDatabase,
) -> Vec<ExplorerGameWithUci> {
    lichess_db
        .games(games.iter().map(|(_, id)| *id))
        .expect("get games")
        .into_iter()
        .zip(games.into_iter())
        .filter_map(|(info, (uci, id))| {
            info.map(|info| ExplorerGameWithUci {
                uci,
                row: ExplorerGame::from_lichess(id, info),
            })
        })
        .collect()
}

struct PlayerStreamState {
    indexing: Option<watch::Receiver<()>>,
    key: KeyPrefix,
    db: Arc<Database>,
    color: Color,
    filter: PlayerQueryFilter,
    limits: Limits,
    pos: VariantPosition,
    opening: Option<&'static Opening>,
    first: bool,
    done: bool,
}

async fn player(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(indexer): State<IndexerStub>,
    Query(query): Query<PlayerQuery>,
) -> Result<NdJson<impl Stream<Item = ExplorerResponse>>, Error> {
    let player = UserId::from(query.player);
    let indexing = indexer.index_player(&player).await;
    let PlayPosition { pos, opening } = query.play.position(openings)?;
    let key = KeyBuilder::player(&player, query.color)
        .with_zobrist(pos.variant(), pos.zobrist_hash(EnPassantMode::Legal));

    let state = PlayerStreamState {
        color: query.color,
        filter: query.filter,
        limits: query.limits,
        db,
        indexing,
        opening,
        key,
        pos,
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

            task::spawn_blocking(move || {
                let lichess_db = state.db.lichess();
                let filtered = lichess_db
                    .read_player(&state.key, state.filter.since, state.filter.until)
                    .expect("read player")
                    .prepare(state.color, &state.filter, &state.limits);

                Some((
                    ExplorerResponse {
                        total: filtered.total,
                        moves: finalize_lichess_moves(filtered.moves, &state.pos, &lichess_db),
                        recent_games: Some(finalize_lichess_games(filtered.recent_games, &lichess_db)),
                        top_games: None,
                        opening: state.opening,
                    },
                    state,
                ))
            }).await.expect("blocking player")
        },
    ).dedup_by_key(|res| res.total.total())))
}

async fn masters_import(
    State(importer): State<MastersImporter>,
    Json(body): Json<MastersGameWithId>,
) -> Result<(), Error> {
    task::spawn_blocking(move || importer.import(body))
        .await
        .expect("blocking masters import")
}

#[serde_as]
#[derive(Deserialize)]
struct MastersGameId(#[serde_as(as = "DisplayFromStr")] GameId);

async fn masters_pgn(
    Path(MastersGameId(id)): Path<MastersGameId>,
    State(db): State<Arc<Database>>,
) -> Result<MastersGame, StatusCode> {
    task::spawn_blocking(
        move || match db.masters().game(id).expect("get masters game") {
            Some(game) => Ok(game),
            None => Err(StatusCode::NOT_FOUND),
        },
    )
    .await
    .expect("blocking masters pgn")
}

async fn masters(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(masters_cache): State<ExplorerCache<MastersQuery>>,
    Query(query): Query<MastersQuery>,
) -> Result<Json<ExplorerResponse>, Error> {
    masters_cache
        .get_with(query.clone(), async move {
            task::spawn_blocking(move || {
                let PlayPosition { pos, opening } = query.play.position(openings)?;
                let key = KeyBuilder::masters()
                    .with_zobrist(pos.variant(), pos.zobrist_hash(EnPassantMode::Legal));
                let masters_db = db.masters();
                let entry = masters_db
                    .read(key, query.since, query.until)
                    .expect("get masters")
                    .prepare(&query.limits);

                Ok(Json(ExplorerResponse {
                    total: entry.total,
                    moves: entry
                        .moves
                        .into_iter()
                        .map(|p| ExplorerMove {
                            san: p.uci.to_move(&pos).map_or(
                                SanPlus {
                                    san: San::Null,
                                    suffix: None,
                                },
                                |m| SanPlus::from_move(pos.clone(), &m),
                            ),
                            uci: p.uci,
                            average_rating: p.average_rating,
                            average_opponent_rating: p.average_opponent_rating,
                            performance: p.performance,
                            stats: p.stats,
                            game: p.game.and_then(|id| {
                                masters_db
                                    .game(id)
                                    .expect("get masters game")
                                    .map(|info| ExplorerGame::from_masters(id, info))
                            }),
                        })
                        .collect(),
                    top_games: Some(
                        masters_db
                            .games(entry.top_games.iter().map(|(_, id)| *id))
                            .expect("get masters games")
                            .into_iter()
                            .zip(entry.top_games.into_iter())
                            .filter_map(|(info, (uci, id))| {
                                info.map(|info| ExplorerGameWithUci {
                                    uci: uci.clone(),
                                    row: ExplorerGame::from_masters(id, info),
                                })
                            })
                            .collect(),
                    ),
                    opening,
                    recent_games: None,
                }))
            })
            .await
            .expect("blocking masters")
        })
        .await
}

async fn lichess_import(
    State(importer): State<LichessImporter>,
    Json(body): Json<Vec<LichessGameImport>>,
) -> Result<(), Error> {
    task::spawn_blocking(move || importer.import_many(body))
        .await
        .expect("blocking lichess import")
}

async fn lichess(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(lichess_cache): State<ExplorerCache<LichessQuery>>,
    Query(query): Query<LichessQuery>,
) -> Result<Json<ExplorerResponse>, Error> {
    lichess_cache
        .get_with(query.clone(), async move {
            task::spawn_blocking(move || {
                let PlayPosition { pos, opening } = query.play.position(openings)?;
                let key = KeyBuilder::lichess()
                    .with_zobrist(pos.variant(), pos.zobrist_hash(EnPassantMode::Legal));
                let lichess_db = db.lichess();
                let filtered = lichess_db
                    .read_lichess(&key, query.filter.since, query.filter.until)
                    .expect("get lichess")
                    .prepare(&query.filter, &query.limits);

                Ok(Json(ExplorerResponse {
                    total: filtered.total,
                    moves: finalize_lichess_moves(filtered.moves, &pos, &lichess_db),
                    recent_games: Some(finalize_lichess_games(filtered.recent_games, &lichess_db)),
                    top_games: Some(finalize_lichess_games(filtered.top_games, &lichess_db)),
                    opening,
                }))
            })
            .await
            .expect("blocking lichess")
        })
        .await
}

async fn lichess_history(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    Query(query): Query<LichessHistoryQuery>,
) -> Result<Json<ExplorerHistoryResponse>, Error> {
    task::spawn_blocking(move || {
        let PlayPosition { pos, opening } = query.play.position(openings)?;
        let key = KeyBuilder::lichess()
            .with_zobrist(pos.variant(), pos.zobrist_hash(EnPassantMode::Legal));
        let lichess_db = db.lichess();
        Ok(Json(ExplorerHistoryResponse {
            history: lichess_db
                .read_lichess_history(&key, &query.filter)
                .expect("get lichess history"),
            opening,
        }))
    })
    .await
    .expect("blocking lichess history")
}
