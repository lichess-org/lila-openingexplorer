use std::{
    net::SocketAddr,
    ops::Deref,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use moka::future::Cache;
use partial_sort::PartialSort;
use rocksdb::DBPinnableSlice;
use shakmaty::{EnPassantMode, fen::Fen, san::SanPlus, variant::VariantPosition};
use tokio::{runtime::Handle, sync::Semaphore, task};

use crate::{
    packed::{
        CellKey, GameId, GameIndex, GroupHeader, InvalidGameId, LichessEntry, PositionKey, Rating,
        unpack_game_value,
    },
    server::{
        blacklist::{self, Blacklist, LilaConfig},
        db::Database,
        explore::{
            GameRef, MASTERS_TOP_CANDIDATES, MAX_MASTERS_GAMES, PreparedMove, prepare_lichess,
            prepare_masters,
        },
        listen,
        metrics::Metrics,
        openings::{self, Openings},
        pgn::masters_pgn,
        query::{LichessQuery, MastersQuery, Play, PlayError, PlayPosition},
        replay::find_visit,
        response::{
            ExplorerGame, ExplorerGameWithUci, ExplorerMove, ExplorerResponse, HistoryEntry, Stats,
        },
    },
    util::{Counter, Exposition},
};

/// Which database the server serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Lichess,
    Masters,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Lichess => "lichess",
            Kind::Masters => "masters",
        }
    }
}

/// A query of either database, as the key of the response cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExploreQuery {
    Lichess(LichessQuery),
    Masters(MastersQuery),
}

impl ExploreQuery {
    fn play(&self) -> &Play {
        match self {
            ExploreQuery::Lichess(query) => &query.play,
            ExploreQuery::Masters(query) => &query.play,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub kind: Kind,
    pub bind: SocketAddr,
    pub admin_bind: Option<SocketAddr>,
    pub cors: bool,
    pub cache_entries: u64,
    pub cache_ttl: Duration,
    pub blocking_threads: usize,
    pub parallel_fetch: bool,
    pub cold: bool,
    pub openings_dir: Option<PathBuf>,
    pub no_openings: bool,
    pub blacklist: Option<LilaConfig>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bad request: {0}")]
    Query(#[from] QueryRejection),
    #[error("bad request: {0}")]
    Play(#[from] PlayError),
    #[error("bad request: {0}")]
    GameId(#[from] InvalidGameId),
    #[error("not found")]
    NotFound,
    #[error("database: {0}")]
    Db(#[from] rocksdb::Error),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Error::Query(_) | Error::Play(_) | Error::GameId(_) => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            Error::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            Error::Db(_) => {
                log::error!("{self}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_owned(),
                )
            }
        }
        .into_response()
    }
}

pub struct AppState {
    config: AppConfig,
    db: Arc<Database>,
    openings: RwLock<Openings>,
    blacklist: Blacklist,
    cache: Cache<ExploreQuery, Bytes>,
    semaphore: Semaphore,
    metrics: Metrics,
}

type Shared = Arc<AppState>;

impl AppState {
    /// The packed entry of a position: from the positions column family,
    /// or (unless disabled) synthesized from the single game that reached
    /// it.
    fn entry(&self, pos: &VariantPosition) -> Result<Option<Entry<'_>>, rocksdb::Error> {
        let hash = PositionKey::new(pos);
        if let Some(entry) = self.db.positions_entry(&hash)? {
            self.metrics.positions_hits.inc();
            return Ok(Some(Entry::Pinned(entry)));
        }
        if !self.config.cold {
            self.metrics.misses.inc();
            return Ok(None);
        }
        let candidates = self.db.cold_candidates(&hash)?;
        if candidates.is_empty() {
            self.metrics.misses.inc();
            return Ok(None);
        }
        let mut entry = LichessEntry::new();
        let mut found = false;
        for (id, game) in candidates
            .iter()
            .zip(self.load_games(&candidates, &self.metrics.cold_games_loaded)?)
        {
            let Some(game) = game else {
                log::warn!("cold candidate {id} not found in games");
                continue;
            };
            let (_, game) = unpack_game_value(&game);
            if let Some(visit) = find_visit(&game, &hash) {
                entry.add_game(
                    CellKey::new(
                        visit.m,
                        GroupHeader {
                            speed: game.speed,
                            rating_group: game.rating_group(),
                        },
                        game.date.earliest_month(),
                    ),
                    game.ratings_or_default()[visit.turn],
                    game.outcome,
                    *id,
                );
                found = true;
            }
        }
        if found {
            self.metrics.cold_hits.inc();
            let mut buf = Vec::new();
            entry.pack_entire(&mut buf);
            Ok(Some(Entry::Owned(buf)))
        } else {
            self.metrics.misses.inc();
            Ok(None)
        }
    }

    /// Loads games by id, keeping the order.
    fn load_games(
        &self,
        ids: &[GameIndex],
        loaded: &Counter,
    ) -> Result<Vec<Option<Vec<u8>>>, rocksdb::Error> {
        loaded.add(ids.len() as u64);
        if !self.config.parallel_fetch {
            return Ok(self
                .db
                .games(ids)?
                .into_iter()
                .map(|game| game.map(|game| game.to_vec()))
                .collect());
        }
        let mut games = Vec::with_capacity(ids.len());
        let mut missing = Vec::new();
        for (i, cached) in self.db.cached_games(ids)?.into_iter().enumerate() {
            match cached {
                Some(game) => games.push(game.map(|game| game.to_vec())),
                None => {
                    games.push(None);
                    missing.push(i);
                }
            }
        }
        self.metrics.parallel_fetch_games.add(missing.len() as u64);
        match missing[..] {
            [] => (),
            [i] => games[i] = self.db.game(ids[i])?.map(|game| game.to_vec()),
            _ => {
                // Called on a thread of the blocking pool (outside of async
                // contexts), so it can wait for the fetches, which run on
                // further threads of the same pool.
                let runtime = Handle::current();
                let fetches: Vec<_> = missing
                    .iter()
                    .map(|&i| {
                        let db = Arc::clone(&self.db);
                        let id = ids[i];
                        let fetch = runtime.spawn_blocking(move || {
                            db.game(id).map(|game| game.map(|game| game.to_vec()))
                        });
                        (i, fetch)
                    })
                    .collect();
                runtime.block_on(async {
                    for (i, fetch) in fetches {
                        games[i] = fetch.await.expect("game fetch")?;
                    }
                    Ok::<_, rocksdb::Error>(())
                })?;
            }
        }
        Ok(games)
    }

    /// Drops the games of blacklisted players, as if they were not found.
    fn hide_blacklisted(&self, games: &mut [Option<Vec<u8>>]) {
        if self.blacklist.is_empty() {
            return;
        }
        for slot in games {
            if let Some(game) = slot {
                let (_, game) = unpack_game_value(game);
                if self
                    .blacklist
                    .contains_any([game.players.white.name, game.players.black.name])
                {
                    self.metrics.blacklisted_games.inc();
                    *slot = None;
                }
            }
        }
    }

    fn explorer_moves(
        &self,
        pos: &VariantPosition,
        moves: Vec<PreparedMove>,
        games: &mut Vec<GameIndex>,
    ) -> Vec<(ExplorerMove, Option<usize>)> {
        let openings = self.openings.read().expect("read openings");
        moves
            .into_iter()
            .filter_map(|pm| {
                let uci = pm.m.unpack();
                let Ok(m) = uci.to_move(pos) else {
                    // Only possible if the key of the position collides
                    // with the key of another position.
                    log::warn!(
                        "skipping illegal move {uci} in {}",
                        Fen::from_position(pos, EnPassantMode::Legal)
                    );
                    return None;
                };
                let mut pos_after = pos.clone();
                let san = SanPlus::from_move_and_play_unchecked(&mut pos_after, m);
                let game = pm.game.map(|id| {
                    games.push(id);
                    games.len() - 1
                });
                Some((
                    ExplorerMove {
                        uci,
                        san,
                        average_rating: pm.stats.average_rating(),
                        stats: Stats::from(&pm.stats),
                        game: None,
                        opening: openings.classify_exact(&pos_after).cloned(),
                    },
                    game,
                ))
            })
            .collect()
    }

    fn with_uci(
        refs: &[GameRef],
        games: &[Option<Vec<u8>>],
        offset: usize,
        masters: bool,
    ) -> Vec<ExplorerGameWithUci> {
        refs.iter()
            .zip(&games[offset..])
            .filter_map(|(game_ref, game)| {
                game.as_ref().map(|game| {
                    let (id, game) = unpack_game_value(game);
                    ExplorerGameWithUci {
                        uci: game_ref.m.unpack(),
                        game: ExplorerGame::new(id, &game, masters),
                    }
                })
            })
            .collect()
    }

    fn lichess(&self, query: &LichessQuery, play: PlayPosition) -> Result<Bytes, rocksdb::Error> {
        let PlayPosition { pos, opening } = play;
        let entry = self.entry(&pos)?;
        let bytes: &[u8] = entry.as_deref().unwrap_or(&[]);
        let prepared = prepare_lichess(bytes, &query.filter, &query.limits, query.history);

        let mut ids = Vec::new();
        let mut moves = self.explorer_moves(&pos, prepared.moves, &mut ids);
        let recent_offset = ids.len();
        ids.extend(prepared.recent_games.iter().map(|g| g.id));
        let top_offset = ids.len();
        ids.extend(prepared.top_games.iter().map(|g| g.id));
        let mut games = self.load_games(&ids, &self.metrics.response_games_loaded)?;
        self.hide_blacklisted(&mut games);
        attach_single_games(&mut moves, &games, false);

        let response = ExplorerResponse {
            total: Stats::from(&prepared.total),
            moves: moves.into_iter().map(|(m, _)| m).collect(),
            recent_games: Some(Self::with_uci(
                &prepared.recent_games,
                &games,
                recent_offset,
                false,
            )),
            top_games: Some(Self::with_uci(
                &prepared.top_games,
                &games,
                top_offset,
                false,
            )),
            opening,
            history: prepared.history.map(|history| {
                history
                    .into_iter()
                    .map(|segment| HistoryEntry {
                        month: segment.month,
                        stats: Stats::from(&segment.stats),
                    })
                    .collect()
            }),
        };
        Ok(Bytes::from(
            serde_json::to_vec(&response).expect("serialize"),
        ))
    }

    fn masters(&self, query: &MastersQuery, play: PlayPosition) -> Result<Bytes, rocksdb::Error> {
        let PlayPosition { pos, opening } = play;
        let entry = self.entry(&pos)?;
        let bytes: &[u8] = entry.as_deref().unwrap_or(&[]);
        let prepared = prepare_masters(
            bytes,
            &query.filter(),
            &query.limits,
            MASTERS_TOP_CANDIDATES,
        );

        let mut ids = Vec::new();
        let mut moves = self.explorer_moves(&pos, prepared.moves, &mut ids);
        let candidates_offset = ids.len();
        ids.extend(prepared.candidates.iter().map(|g| g.id));
        let games = self.load_games(&ids, &self.metrics.response_games_loaded)?;
        attach_single_games(&mut moves, &games, true);

        // Rank the candidates by their actual ratings.
        let mut top_games: Vec<(i32, usize)> = prepared
            .candidates
            .iter()
            .zip(&games[candidates_offset..])
            .enumerate()
            .filter_map(|(i, (_, game))| {
                game.as_ref().map(|game| {
                    let (_, game) = unpack_game_value(game);
                    let rating = |rating: Option<Rating>| rating.map_or(0, i32::from);
                    (
                        rating(game.players.white.rating) + rating(game.players.black.rating),
                        i,
                    )
                })
            })
            .collect();
        let num = query
            .limits
            .top_games
            .min(MAX_MASTERS_GAMES)
            .min(top_games.len());
        top_games.partial_sort(num, |(a, _), (b, _)| b.cmp(a));
        top_games.truncate(num);
        let top_games = top_games
            .into_iter()
            .map(|(_, i)| {
                let game_ref = &prepared.candidates[i];
                let (id, game) =
                    unpack_game_value(games[candidates_offset + i].as_deref().expect("loaded"));
                ExplorerGameWithUci {
                    uci: game_ref.m.unpack(),
                    game: ExplorerGame::new(id, &game, true),
                }
            })
            .collect();

        let response = ExplorerResponse {
            total: Stats::from(&prepared.total),
            moves: moves.into_iter().map(|(m, _)| m).collect(),
            recent_games: None,
            top_games: Some(top_games),
            opening,
            history: None,
        };
        Ok(Bytes::from(
            serde_json::to_vec(&response).expect("serialize"),
        ))
    }

    fn explore(&self, query: &ExploreQuery, play: PlayPosition) -> Result<Bytes, rocksdb::Error> {
        match query {
            ExploreQuery::Lichess(query) => self.lichess(query, play),
            ExploreQuery::Masters(query) => self.masters(query, play),
        }
    }

    fn pgn(&self, id: GameId) -> Result<String, Error> {
        self.metrics.pgn_requests.inc();
        let index = self.db.game_index(id)?.ok_or(Error::NotFound)?;
        let game = self.db.game(index)?.ok_or(Error::NotFound)?;
        let (id, game) = unpack_game_value(&game);
        Ok(masters_pgn(id, &game))
    }
}

/// A packed entry, borrowed from the block cache or synthesized.
enum Entry<'a> {
    Pinned(DBPinnableSlice<'a>),
    Owned(Vec<u8>),
}

impl Deref for Entry<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Entry::Pinned(slice) => slice,
            Entry::Owned(vec) => vec,
        }
    }
}

/// Attaches the only game of each move, once the games are loaded.
fn attach_single_games(
    moves: &mut [(ExplorerMove, Option<usize>)],
    games: &[Option<Vec<u8>>],
    masters: bool,
) {
    for (m, index) in moves {
        if let Some(i) = *index
            && let Some(game) = &games[i]
        {
            let (id, game) = unpack_game_value(game);
            m.game = Some(ExplorerGame::new(id, &game, masters));
        }
    }
}

async fn blocking<T: Send + 'static>(
    state: &Shared,
    f: impl FnOnce(&Shared) -> T + Send + 'static,
) -> T {
    let _permit = state
        .semaphore
        .acquire()
        .await
        .expect("semaphore not closed");
    let state = Arc::clone(state);
    task::spawn_blocking(move || f(&state))
        .await
        .expect("blocking task")
}

fn json(bytes: Bytes) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        bytes,
    )
        .into_response()
}

/// Maps a rejected query string to a bad request.
fn parse_query<T>(state: &AppState, query: Result<Query<T>, QueryRejection>) -> Result<T, Error> {
    query.map(|Query(query)| query).map_err(|err| {
        state.metrics.bad_requests.inc();
        Error::from(err)
    })
}

async fn lichess(
    State(state): State<Shared>,
    query: Result<Query<LichessQuery>, QueryRejection>,
) -> Result<Response, Error> {
    let query = parse_query(&state, query)?;
    explore(state, ExploreQuery::Lichess(query)).await
}

async fn masters(
    State(state): State<Shared>,
    query: Result<Query<MastersQuery>, QueryRejection>,
) -> Result<Response, Error> {
    let query = parse_query(&state, query)?;
    explore(state, ExploreQuery::Masters(query)).await
}

async fn explore(state: Shared, query: ExploreQuery) -> Result<Response, Error> {
    let started = Instant::now();
    let play = query
        .play()
        .position(&state.openings.read().expect("read openings"))
        .inspect_err(|_| state.metrics.bad_requests.inc())?;
    state.metrics.requests.inc();
    let state2 = Arc::clone(&state);
    let bytes = state
        .cache
        .try_get_with(query.clone(), async move {
            state2.metrics.computed.inc();
            let bytes = blocking(&state2, move |state| {
                let started = Instant::now();
                let bytes = state.explore(&query, play);
                state.metrics.compute.record(started.elapsed());
                bytes
            })
            .await;
            state2.metrics.request.record(started.elapsed());
            bytes
        })
        .await
        .map_err(|err| Error::Db((*err).clone()))?;
    Ok(json(bytes))
}

async fn masters_pgn_handler(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let id: GameId = id.parse()?;
    let pgn = blocking(&state, move |state| state.pgn(id)).await?;
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-chess-pgn"),
        )],
        pgn,
    )
        .into_response())
}

async fn metrics(State(state): State<Shared>) -> Response {
    let mut out = Exposition::new();
    state.metrics.write(state.config.kind, &mut out);
    out.gauge(
        "response_cache_entries",
        "Entries in the response cache.",
        state.cache.entry_count(),
    );
    out.gauge(
        "block_cache_usage_bytes",
        "Memory used by the RocksDB block cache.",
        state.db.cache_usage(),
    );
    out.gauge(
        "block_cache_pinned_usage_bytes",
        "Memory used by pinned entries of the RocksDB block cache.",
        state.db.cache_pinned_usage(),
    );
    out.gauge(
        "openings",
        "Known opening names.",
        state.openings.read().expect("read openings").len(),
    );
    if state.config.blacklist.is_some() {
        out.gauge(
            "blacklisted_players",
            "Players on the blacklist.",
            state.blacklist.len(),
        );
    }
    write_tickers(&state.db, &mut out);
    listen::metrics_response(out)
}

/// RocksDB statistics, if collected (not --no-db-statistics).
fn write_tickers(db: &Database, out: &mut Exposition) {
    use rocksdb::statistics::Ticker;
    if let Some(hits) = db.ticker(Ticker::BlockCacheHit) {
        out.counter(
            "rocksdb_block_cache_hits_total",
            "RocksDB block cache hits.",
            hits,
        );
    }
    out.labeled_counter(
        "rocksdb_block_cache_misses_total",
        "RocksDB block cache misses, by block type.",
        "block",
        [
            ("index", Ticker::BlockCacheIndexMiss),
            ("filter", Ticker::BlockCacheFilterMiss),
            ("data", Ticker::BlockCacheDataMiss),
        ]
        .into_iter()
        .filter_map(|(block, ticker)| Some((block, db.ticker(ticker)?))),
    );
    if let Some(useful) = db.ticker(Ticker::BloomFilterUseful) {
        out.counter(
            "rocksdb_bloom_filter_useful_total",
            "Point lookups that a filter ruled out.",
            useful,
        );
    }
}

async fn db_prop(State(state): State<Shared>, Path(prop): Path<String>) -> Result<String, Error> {
    blocking(&state, move |state| {
        state.db.property(&prop)?.ok_or(Error::NotFound)
    })
    .await
}

async fn cf_prop(
    State(state): State<Shared>,
    Path((cf, prop)): Path<(String, String)>,
) -> Result<String, Error> {
    blocking(&state, move |state| {
        state.db.cf_property(&cf, &prop)?.ok_or(Error::NotFound)
    })
    .await
}

pub async fn serve(config: AppConfig, db: Database) -> Result<(), Box<dyn std::error::Error>> {
    let openings = Openings::initial(config.openings_dir.as_deref())?;

    let state: Shared = Arc::new(AppState {
        cache: Cache::builder()
            .max_capacity(config.cache_entries)
            .time_to_live(config.cache_ttl)
            .build(),
        semaphore: Semaphore::new(config.blocking_threads),
        openings: RwLock::new(openings),
        blacklist: Blacklist::default(),
        metrics: Metrics::default(),
        config,
        db: Arc::new(db),
    });

    if state.config.openings_dir.is_none() && !state.config.no_openings {
        let state = Arc::clone(&state);
        task::spawn(async move {
            openings::periodic_refresh(&state.openings, || state.cache.invalidate_all()).await;
        });
    }

    if let Some(lila) = state.config.blacklist.clone() {
        let state = Arc::clone(&state);
        task::spawn(async move {
            blacklist::periodic_update(&state.blacklist, lila, || state.cache.invalidate_all())
                .await
        });
    }

    let app = Router::new();
    let app = match state.config.kind {
        Kind::Lichess => app.route("/lichess", get(lichess)),
        Kind::Masters => app
            .route("/masters", get(masters))
            .route("/masters/pgn/{id}", get(masters_pgn_handler)),
    };
    let app = app.with_state(Arc::clone(&state));
    let app = if state.config.cors {
        app.layer(listen::cors())
    } else {
        app
    };

    let admin = state.config.admin_bind.map(|addr| {
        let router = Router::new()
            .route("/metrics", get(metrics))
            .route("/db/{prop}", get(db_prop))
            .route("/cf/{cf}/{prop}", get(cf_prop))
            .with_state(Arc::clone(&state));
        (addr, router)
    });

    listen::serve_until_signal(state.config.bind, app, admin).await?;
    Ok(())
}
