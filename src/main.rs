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
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use clap::Parser;
use futures_util::stream::Stream;
use moka::sync::Cache;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use shakmaty::{
    san::{San, SanPlus},
    uci::Uci,
    variant::VariantPosition,
    Color,
};
use tikv_jemallocator::Jemalloc;
use tokio::sync::watch;
use tower::ServiceBuilder;

use crate::{
    api::{
        Error, ExplorerGame, ExplorerGameWithUci, ExplorerMove, ExplorerResponse, LichessQuery,
        Limits, MastersQuery, NdJson, PlayPosition, PlayerQuery, PlayerQueryFilter,
    },
    db::{Database, LichessDatabase},
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
    #[clap(long, default_value = "127.0.0.1:9002")]
    bind: SocketAddr,
    /// Path to RocksDB database
    #[clap(long, default_value = "_db")]
    db: PathBuf,
    /// Allow access from all origins.
    #[clap(long)]
    cors: bool,
    #[clap(flatten)]
    indexer: IndexerOpt,
    #[clap(long, default_value = "2000")]
    cache_size: u64,
}

type ExplorerCache<T> = Cache<T, Result<Json<ExplorerResponse>, Error>>;

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
    let masters_importer = MastersImporter::new(Arc::clone(&db));
    let lichess_importer = LichessImporter::new(Arc::clone(&db));

    let lichess_cache: ExplorerCache<LichessQuery> = Cache::builder()
        .max_capacity(opt.cache_size)
        .time_to_live(Duration::from_secs(5 * 60))
        .build();

    let masters_cache: ExplorerCache<MastersQuery> = Cache::builder()
        .max_capacity(opt.cache_size)
        .time_to_live(Duration::from_secs(5 * 60))
        .build();

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
        .route("/player", get(player))
        .route("/master/pgn/:id", get(masters_pgn)) // bc
        .route("/master", get(masters)) // bc
        .route("/personal", get(player)) // bc
        .layer(
            ServiceBuilder::new()
                .layer(Extension(openings))
                .layer(Extension(db))
                .layer(Extension(masters_cache))
                .layer(Extension(lichess_cache))
                .layer(Extension(masters_importer))
                .layer(Extension(lichess_importer))
                .layer(Extension(indexer)),
        );

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
    Extension(db): Extension<Arc<Database>>,
) -> Result<String, StatusCode> {
    db.inner
        .cf_handle(&path.cf)
        .and_then(|cf| {
            db.inner
                .property_value_cf(cf, &path.prop)
                .expect("property value")
        })
        .ok_or(StatusCode::NOT_FOUND)
}

async fn db_prop(
    Path(prop): Path<String>,
    Extension(db): Extension<Arc<Database>>,
) -> Result<String, StatusCode> {
    db.inner
        .property_value(&prop)
        .expect("property value")
        .ok_or(StatusCode::NOT_FOUND)
}

async fn num_indexing(Extension(indexer): Extension<IndexerStub>) -> String {
    indexer.num_indexing().await.to_string()
}

async fn compact(Extension(db): Extension<Arc<Database>>) {
    db.compact();
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
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Extension(indexer): Extension<IndexerStub>,
    Query(query): Query<PlayerQuery>,
) -> Result<NdJson<impl Stream<Item = ExplorerResponse>>, Error> {
    let player = UserId::from(query.player);
    let indexing = indexer.index_player(&player).await;
    let PlayPosition {
        variant,
        pos,
        opening,
    } = query.play.position(openings)?;
    let key = KeyBuilder::player(&player, query.color).with_zobrist(variant, pos.zobrist_hash());

    let state = PlayerStreamState {
        color: query.color,
        filter: query.filter,
        limits: query.limits,
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
        },
    ).dedup_by_key(|res| res.total.total())))
}

async fn masters_import(
    Json(body): Json<MastersGameWithId>,
    Extension(importer): Extension<MastersImporter>,
) -> Result<(), Error> {
    importer.import(body).await
}

#[serde_as]
#[derive(Deserialize)]
struct MastersGameId(#[serde_as(as = "DisplayFromStr")] GameId);

async fn masters_pgn(
    Path(MastersGameId(id)): Path<MastersGameId>,
    Extension(db): Extension<Arc<Database>>,
) -> Result<MastersGame, StatusCode> {
    match db.masters().game(id).expect("get masters game") {
        Some(game) => Ok(game),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn masters(
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Extension(masters_cache): Extension<ExplorerCache<MastersQuery>>,
    Query(query): Query<MastersQuery>,
) -> Result<Json<ExplorerResponse>, Error> {
    masters_cache.get_with(query.clone(), || {
        let PlayPosition {
            variant,
            pos,
            opening,
        } = query.play.position(openings)?;
        let key = KeyBuilder::masters().with_zobrist(variant, pos.zobrist_hash());
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
}

async fn lichess_import(
    Json(body): Json<Vec<LichessGameImport>>,
    Extension(importer): Extension<LichessImporter>,
) -> Result<(), Error> {
    for game in body {
        importer.import(game).await?;
    }
    Ok(())
}

async fn lichess(
    Extension(openings): Extension<&'static Openings>,
    Extension(db): Extension<Arc<Database>>,
    Extension(lichess_cache): Extension<ExplorerCache<LichessQuery>>,
    Query(query): Query<LichessQuery>,
) -> Result<Json<ExplorerResponse>, Error> {
    lichess_cache.get_with(query.clone(), || {
        let PlayPosition {
            variant,
            pos,
            opening,
        } = query.play.position(openings)?;
        let key = KeyBuilder::lichess().with_zobrist(variant, pos.zobrist_hash());
        let lichess_db = db.lichess();
        let filtered = lichess_db
            .read_lichess(&key, query.filter.since, query.filter.until)
            .expect("get lichess")
            .prepare(&query.filter, &query.limits);

        Ok(Json(ExplorerResponse {
            total: filtered.total,
            moves: finalize_lichess_moves(filtered.moves, pos.as_inner(), &lichess_db),
            recent_games: Some(finalize_lichess_games(filtered.recent_games, &lichess_db)),
            top_games: Some(finalize_lichess_games(filtered.top_games, &lichess_db)),
            opening,
        }))
    })
}
