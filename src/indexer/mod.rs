use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use async_channel::TrySendError;
use axum::http::StatusCode;
use clap::Clap;
use futures_util::StreamExt;
use rustc_hash::FxHashMap;
use shakmaty::{
    uci::Uci, variant::VariantPosition, zobrist::Zobrist, ByColor, CastlingMode, Outcome, Position,
};
use tokio::{
    sync::{watch, RwLock},
    task::JoinHandle,
};

use crate::{
    db::Database,
    model::{
        GameInfo, GameInfoPlayer, Mode, Month, PersonalEntry, PersonalKeyBuilder, PersonalStatus,
        UserId,
    },
};

mod lila;

use lila::{Game, Lila};

const MAX_PLIES: usize = 50;

#[derive(Clap, Clone)]
pub struct IndexerOpt {
    #[clap(long = "lila", default_value = "https://lichess.org")]
    lila: String,
    #[clap(long = "bearer")]
    bearer: Option<String>,
    #[clap(long = "indexers", default_value = "16")]
    indexers: usize,
}

#[derive(Clone)]
pub struct IndexerStub {
    db: Arc<Database>,
    indexing: Arc<RwLock<HashMap<UserId, watch::Sender<()>>>>,
    tx: async_channel::Sender<IndexerMessage>,
}

impl IndexerStub {
    pub fn spawn(db: Arc<Database>, opt: IndexerOpt) -> (IndexerStub, Vec<JoinHandle<()>>) {
        let indexing = Arc::new(RwLock::new(HashMap::new()));

        let (tx, rx) = async_channel::bounded(1000);
        let mut join_handles = Vec::with_capacity(opt.indexers);
        for idx in 0..opt.indexers {
            join_handles.push(tokio::spawn(
                IndexerActor {
                    idx,
                    rx: rx.clone(),
                    indexing: Arc::clone(&indexing),
                    db: Arc::clone(&db),
                    lila: Lila::new(opt.clone()),
                }
                .run(),
            ));
        }

        (IndexerStub { db, indexing, tx }, join_handles)
    }

    pub async fn num_indexing(&self) -> usize {
        let guard = self.indexing.read().await;
        guard.len()
    }

    pub async fn index_player(&self, player: &UserId) -> Option<watch::Receiver<()>> {
        // Optimization: First try subscribing to an existing indexing run,
        // without acquiring a write lock.
        {
            let guard = self.indexing.read().await;
            if let Some(sender) = guard.get(player) {
                return Some(sender.subscribe());
            }
        }

        // Check player indexing status.
        let mut status = self
            .db
            .queryable()
            .get_player_status(player)
            .expect("get player status")
            .unwrap_or_default();

        let since_created_at = match status
            .maybe_revisit_ongoing()
            .or_else(|| status.maybe_index())
        {
            Some(since) => since,
            None => return None, // Do not reindex so soon!
        };

        // Queue indexing request.
        let mut guard = self.indexing.write().await;
        let entry = match guard.entry(player.to_owned()) {
            Entry::Occupied(entry) => return Some(entry.get().subscribe()),
            Entry::Vacant(entry) => entry,
        };

        match self.tx.try_send(IndexerMessage::IndexPlayer {
            player: player.to_owned(),
            status,
            since_created_at,
        }) {
            Ok(_) => {
                let (sender, receiver) = watch::channel(());
                entry.insert(sender);
                Some(receiver)
            }
            Err(TrySendError::Full(_)) => {
                log::error!(
                    "not queuing {} because indexer queue is full",
                    player.as_str()
                );
                None
            }
            Err(TrySendError::Closed(_)) => panic!("all indexers died"),
        }
    }
}

struct IndexerActor {
    idx: usize,
    indexing: Arc<RwLock<HashMap<UserId, watch::Sender<()>>>>,
    rx: async_channel::Receiver<IndexerMessage>,
    db: Arc<Database>,
    lila: Lila,
}

impl IndexerActor {
    async fn run(self) {
        while let Ok(msg) = self.rx.recv().await {
            match msg {
                IndexerMessage::IndexPlayer {
                    player,
                    status,
                    since_created_at,
                } => {
                    self.index_player(&player, status, since_created_at).await;

                    let mut guard = self.indexing.write().await;
                    guard.remove(&player);
                }
            }
        }
    }

    async fn index_player(
        &self,
        player: &UserId,
        mut status: PersonalStatus,
        since_created_at: u64,
    ) {
        let started_at = Instant::now();
        log::info!(
            "indexer {:02}: starting {} (created_at >= {})",
            self.idx,
            player.as_str(),
            since_created_at
        );

        let mut games = match self.lila.user_games(player, since_created_at).await {
            Ok(games) => games,
            Err(err) if err.status() == Some(StatusCode::NOT_FOUND) => {
                log::warn!(
                    "indexer {:02}: did not find player {}",
                    self.idx,
                    player.as_str()
                );
                return;
            }
            Err(err) => {
                log::error!("indexer {:02}: request failed: {}", self.idx, err);
                return;
            }
        };

        let hash = ByColor::new_with(|color| PersonalKeyBuilder::with_user_pov(player, color));
        let mut num_games = 0;
        while let Some(game) = games.next().await {
            let game = match game {
                Ok(game) => game,
                Err(err) => {
                    log::error!("indexer {:02}: {}", self.idx, err);
                    continue;
                }
            };

            self.index_game(player, &hash, game, &mut status);

            num_games += 1;
            if num_games % 1024 == 0 {
                log::info!(
                    "indexer {:02}: indexed {} games for {} ...",
                    self.idx,
                    num_games,
                    player.as_str()
                );
            }
        }

        status.indexed_at = SystemTime::now();
        self.db
            .queryable()
            .put_player_status(player, status)
            .expect("put player status");

        let elapsed = started_at.elapsed();
        log::info!(
            "indexer {:02}: finished {} games for {} in {:.3?} ({:.3?}/game, {:.1} games/s)",
            self.idx,
            num_games,
            player.as_str(),
            elapsed,
            (elapsed.as_nanos() as u64)
                .checked_div(num_games)
                .map(Duration::from_nanos)
                .unwrap_or_default(),
            (num_games as f64) / elapsed.as_secs_f64()
        );
    }

    fn index_game(
        &self,
        player: &UserId,
        hash: &ByColor<PersonalKeyBuilder>,
        game: Game,
        status: &mut PersonalStatus,
    ) {
        status.latest_created_at = game.created_at;

        if game.status.is_ongoing() {
            if status.revisit_ongoing_created_at.is_none() {
                log::debug!(
                    "indexer {:02}: will revisit ongoing game {} eventually",
                    self.idx,
                    game.id
                );
                status.revisit_ongoing_created_at = Some(game.created_at);
            }
            return;
        }

        if game.status.is_unindexable() {
            return;
        }

        if game.players.any(|p| p.user.is_none()) {
            return;
        }

        let color = match game
            .players
            .find(|p| p.user.as_ref().map_or(false, |user| user.name == *player))
        {
            Some(color) => color,
            None => {
                log::error!(
                    "indexer {:02}: {} did not play in {}",
                    self.idx,
                    player.as_str(),
                    game.id
                );
                return;
            }
        };

        // Skip game if already indexed from this side. This cannot race with
        // writes, because all writes for the same player are sequenced by
        // this actor. So making a transaction is not required.
        let queryable = self.db.queryable();
        if queryable
            .get_game_info(game.id)
            .expect("get game info")
            .map_or(false, |info| info.indexed.into_color(color))
        {
            log::debug!(
                "indexer {:02}: {}/{} already indexed",
                self.idx,
                game.id,
                color
            );
            return;
        }

        // Prepare basic information and setup initial position.
        let month = Month::from_time_saturating(game.last_move_at);
        let outcome = Outcome::from_winner(game.winner);
        let variant = game.variant.into();
        let pos = match game.initial_fen {
            Some(fen) => VariantPosition::from_setup(variant, &fen, CastlingMode::Chess960),
            None => Ok(VariantPosition::new(variant)),
        };

        let mut pos: Zobrist<_, u128> = match pos {
            Ok(pos) => Zobrist::new(pos),
            Err(err) => {
                log::warn!("indexer {:02}: not indexing {}: {}", self.idx, game.id, err);
                return;
            }
        };

        // Build an intermediate table to remove loops (due to repetitions).
        let mut table: FxHashMap<u128, Uci> =
            FxHashMap::with_capacity_and_hasher(game.moves.len(), Default::default());

        for (ply, san) in game.moves.into_iter().enumerate() {
            if ply >= MAX_PLIES {
                break;
            }

            let m = match san.to_move(&pos) {
                Ok(m) => m,
                Err(err) => {
                    log::warn!(
                        "indexer {:02}: cutting off {} at ply {}: {}: {}",
                        self.idx,
                        game.id,
                        ply,
                        err,
                        san
                    );
                    break;
                }
            };

            let uci = m.to_uci(CastlingMode::Chess960);
            table.insert(pos.zobrist_hash(), uci);

            pos.play_unchecked(&m);
        }

        // Write to database. All writes regarding this game are batched and
        // atomically committed, so the database will always be in a consistent
        // state.
        let mut batch = queryable.batch();

        batch.merge_game_info(
            game.id,
            GameInfo {
                winner: outcome.winner(),
                speed: game.speed,
                mode: Mode::from_rated(game.rated),
                month,
                players: game.players.map(|p| GameInfoPlayer {
                    name: p.user.map(|p| p.name.to_string()),
                    rating: p.rating,
                }),
                indexed: ByColor::new_with(|c| color == c),
            },
        );

        for (zobrist, uci) in table {
            batch.merge_personal(
                hash.by_color(color)
                    .with_zobrist(variant, zobrist)
                    .with_month(month),
                PersonalEntry::new_single(
                    uci.clone(),
                    game.speed,
                    Mode::from_rated(game.rated),
                    game.id,
                    outcome,
                ),
            );
        }

        batch.write().expect("atomically commit game and moves");
    }
}

enum IndexerMessage {
    IndexPlayer {
        player: UserId,
        status: PersonalStatus,
        since_created_at: u64,
    },
}
