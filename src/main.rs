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
use tokio::sync::{watch, Semaphore};

use crate::{
    api::{
        Error, ExplorerGame, ExplorerGameWithUci, ExplorerHistoryResponse, ExplorerMove,
        ExplorerResponse, LichessHistoryQuery, LichessQuery, MastersQuery, NdJson, PlayPosition,
        PlayerLimits, PlayerQuery, PlayerQueryFilter,
    },
    db::{Database, DbOpt, LichessDatabase, LichessStats, MastersStats},
    importer::{LichessGameImport, LichessImporter, MastersImporter},
    indexer::{IndexerOpt, IndexerStub},
    model::{GameId, KeyBuilder, KeyPrefix, MastersGame, MastersGameWithId, PreparedMove, UserId},
    opening::{Opening, Openings},
    util::{spawn_blocking, DedupStreamExt as _},
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
    /// Maximum number of cached responses for /masters.
    #[arg(long, default_value = "64000")]
    masters_cache: u64,
    /// Maximum number of cached responses for /lichess.
    #[arg(long, default_value = "64000")]
    lichess_cache: u64,
    /// Maximum number of cached responses for /lichess/history.
    #[arg(long, default_value = "32000")]
    lichess_history_cache: u64,
    #[command(flatten)]
    db: DbOpt,
    #[command(flatten)]
    indexer: IndexerOpt,
}

type ExplorerCache<T> = Cache<T, Result<Json<ExplorerResponse>, Error>>;

type ExplorerHistoryCache =
    Cache<LichessHistoryQuery, Result<Json<ExplorerHistoryResponse>, Error>>;

#[derive(Clone)]
struct AppState {
    openings: &'static Openings,
    db: Arc<Database>,
    lichess_cache: ExplorerCache<LichessQuery>,
    lichess_history_cache: ExplorerHistoryCache,
    masters_cache: ExplorerCache<MastersQuery>,
    lichess_importer: LichessImporter,
    masters_importer: MastersImporter,
    indexer: IndexerStub,
    semaphore: Arc<Semaphore>,
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

impl FromRef<AppState> for ExplorerHistoryCache {
    fn from_ref(state: &AppState) -> ExplorerHistoryCache {
        state.lichess_history_cache.clone()
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

impl FromRef<AppState> for Arc<Semaphore> {
    fn from_ref(state: &AppState) -> Arc<Semaphore> {
        Arc::clone(&state.semaphore)
    }
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .filter("EXPLORER_LOG")
            .write_style("EXPLORER_LOG_STYLE"),
    )
    .format_timestamp(None)
    .format_module_path(false)
    .format_target(false)
    .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(32)
        .build()
        .expect("tokio runtime")
        .block_on(serve());
}

async fn serve() {
    let opt = Opt::parse();

    let db = Arc::new(Database::open(opt.db).expect("db"));
    let (indexer, join_handles) = IndexerStub::spawn(Arc::clone(&db), opt.indexer);

    let app = Router::new()
        .route("/monitor/cf/:cf/:prop", get(cf_prop))
        .route("/monitor/db/:prop", get(db_prop))
        .route("/monitor", get(monitor))
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
                .max_capacity(opt.lichess_cache)
                .time_to_live(Duration::from_secs(60 * 60 * 12))
                .time_to_idle(Duration::from_secs(60 * 10))
                .build(),
            lichess_history_cache: Cache::builder()
                .max_capacity(opt.lichess_history_cache)
                .time_to_live(Duration::from_secs(60 * 60 * 24))
                .time_to_idle(Duration::from_secs(60 * 60 * 2))
                .build(),
            masters_cache: Cache::builder()
                .max_capacity(opt.masters_cache)
                .time_to_live(Duration::from_secs(60 * 60 * 24))
                .time_to_idle(Duration::from_secs(60 * 10))
                .build(),
            lichess_importer: LichessImporter::new(Arc::clone(&db)),
            masters_importer: MastersImporter::new(Arc::clone(&db)),
            indexer,
            db,
            semaphore: Arc::new(Semaphore::new(32)),
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

#[axum::debug_handler(state = AppState)]
async fn cf_prop(
    Path(path): Path<ColumnFamilyProp>,
    State(db): State<Arc<Database>>,
    State(semaphore): State<Arc<Semaphore>>,
) -> Result<String, StatusCode> {
    spawn_blocking(semaphore.acquire().await.unwrap(), move || {
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
}

#[axum::debug_handler(state = AppState)]
async fn db_prop(
    Path(prop): Path<String>,
    State(db): State<Arc<Database>>,
    State(semaphore): State<Arc<Semaphore>>,
) -> Result<String, StatusCode> {
    spawn_blocking(semaphore.acquire().await.unwrap(), move || {
        db.inner
            .property_value(&prop)
            .expect("property value")
            .ok_or(StatusCode::NOT_FOUND)
    })
    .await
}

#[axum::debug_handler(state = AppState)]
async fn monitor(
    State(lichess_cache): State<ExplorerCache<LichessQuery>>,
    State(lichess_history_cache): State<ExplorerHistoryCache>,
    State(masters_cache): State<ExplorerCache<MastersQuery>>,
    State(indexer): State<IndexerStub>,
    State(db): State<Arc<Database>>,
    State(semaphore): State<Arc<Semaphore>>,
) -> String {
    let num_indexing = indexer.num_indexing().await;
    let num_lichess_cache = lichess_cache.entry_count();
    let num_lichess_history_cache = lichess_history_cache.entry_count();
    let num_masters_cache = masters_cache.entry_count();

    spawn_blocking(semaphore.acquire().await.unwrap(), move || {
        let MastersStats {
            num_masters,
            num_masters_game,
        } = db.masters().estimate_stats().expect("masters stats");

        let LichessStats {
            num_lichess,
            num_lichess_game,
            num_player,
            num_player_status,
        } = db.lichess().estimate_stats().expect("lichess stats");

        format!("opening_explorer indexing={num_indexing}u,lichess_cache={num_lichess_cache}u,lichess_history_cache={num_lichess_history_cache}u,masters_cache={num_masters_cache}u,masters={num_masters}u,masters_game={num_masters_game}u,lichess={num_lichess}u,lichess_game={num_lichess_game}u,player={num_player}u,player_status={num_player_status}u")
    }).await
}

#[axum::debug_handler(state = AppState)]
async fn compact(State(db): State<Arc<Database>>, State(semaphore): State<Arc<Semaphore>>) {
    spawn_blocking(semaphore.acquire().await.unwrap(), move || db.compact()).await
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
    semaphore: Arc<Semaphore>,
    indexing: Option<watch::Receiver<()>>,
    key: KeyPrefix,
    db: Arc<Database>,
    color: Color,
    filter: PlayerQueryFilter,
    limits: PlayerLimits,
    pos: VariantPosition,
    opening: Option<&'static Opening>,
    first: bool,
    done: bool,
}

#[axum::debug_handler(state = AppState)]
async fn player(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(indexer): State<IndexerStub>,
    State(semaphore): State<Arc<Semaphore>>,
    Query(query): Query<PlayerQuery>,
) -> Result<NdJson<impl Stream<Item = ExplorerResponse>>, Error> {
    let player = UserId::from(query.player);
    let indexing = indexer.index_player(&player, &semaphore).await;
    let PlayPosition { pos, opening } = query.play.position(openings)?;
    let key = KeyBuilder::player(&player, query.color)
        .with_zobrist(pos.variant(), pos.zobrist_hash(EnPassantMode::Legal));

    let state = PlayerStreamState {
        semaphore,
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

            let semaphore = Arc::clone(&state.semaphore);
            spawn_blocking(semaphore.acquire().await.unwrap(), move || {
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
            }).await
        },
    ).dedup_by_key(|res| res.total.total())))
}

#[axum::debug_handler(state = AppState)]
async fn masters_import(
    State(importer): State<MastersImporter>,
    State(semaphore): State<Arc<Semaphore>>,
    Json(body): Json<MastersGameWithId>,
) -> Result<(), Error> {
    spawn_blocking(semaphore.acquire().await.unwrap(), move || {
        importer.import(body)
    })
    .await
}

#[serde_as]
#[derive(Deserialize)]
struct MastersGameId(#[serde_as(as = "DisplayFromStr")] GameId);

#[axum::debug_handler(state = AppState)]
async fn masters_pgn(
    Path(MastersGameId(id)): Path<MastersGameId>,
    State(db): State<Arc<Database>>,
    State(semaphore): State<Arc<Semaphore>>,
) -> Result<MastersGame, StatusCode> {
    spawn_blocking(semaphore.acquire().await.unwrap(), move || {
        match db.masters().game(id).expect("get masters game") {
            Some(game) => Ok(game),
            None => Err(StatusCode::NOT_FOUND),
        }
    })
    .await
}

#[axum::debug_handler(state = AppState)]
async fn masters(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(masters_cache): State<ExplorerCache<MastersQuery>>,
    State(semaphore): State<Arc<Semaphore>>,
    Query(query): Query<MastersQuery>,
) -> Result<Json<ExplorerResponse>, Error> {
    let permit = semaphore.acquire().await.unwrap(); // Early, so cancelling cache population is unlikely
    masters_cache
        .get_with(query.clone(), async move {
            spawn_blocking(permit, move || {
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
        })
        .await
}

#[axum::debug_handler(state = AppState)]
async fn lichess_import(
    State(importer): State<LichessImporter>,
    State(semaphore): State<Arc<Semaphore>>,
    Json(body): Json<Vec<LichessGameImport>>,
) -> Result<(), Error> {
    spawn_blocking(semaphore.acquire().await.unwrap(), move || {
        importer.import_many(body)
    })
    .await
}

#[axum::debug_handler(state = AppState)]
async fn lichess(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(lichess_cache): State<ExplorerCache<LichessQuery>>,
    State(semaphore): State<Arc<Semaphore>>,
    Query(query): Query<LichessQuery>,
) -> Result<Json<ExplorerResponse>, Error> {
    let permit = semaphore.acquire().await.unwrap(); // Early, so cancelling cache population is unlikely
    lichess_cache
        .get_with(query.clone(), async move {
            spawn_blocking(permit, move || {
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
        })
        .await
}

#[axum::debug_handler(state = AppState)]
async fn lichess_history(
    State(openings): State<&'static Openings>,
    State(db): State<Arc<Database>>,
    State(lichess_history_cache): State<ExplorerHistoryCache>,
    State(semaphore): State<Arc<Semaphore>>,
    Query(query): Query<LichessHistoryQuery>,
) -> Result<Json<ExplorerHistoryResponse>, Error> {
    let permit = semaphore.acquire().await.unwrap(); // Early, so cancelling cache population is unlikely
    lichess_history_cache
        .get_with(query.clone(), async move {
            spawn_blocking(permit, move || {
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
        })
        .await
}
