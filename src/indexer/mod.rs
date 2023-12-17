use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use clap::Parser;
use futures_util::StreamExt;
use nohash_hasher::IntMap;
use reqwest::StatusCode;
use shakmaty::{
    uci::Uci,
    variant::VariantPosition,
    zobrist::{Zobrist128, ZobristHash},
    ByColor, CastlingMode, Outcome, Position,
};
use tokio::{
    sync::{mpsc, Semaphore},
    task,
    task::JoinHandle,
    time::{sleep, timeout},
};

use crate::{
    db::Database,
    model::{GamePlayer, KeyBuilder, LichessGame, Mode, Month, PlayerEntry, PlayerStatus, UserId},
    util::spawn_blocking,
};

mod lila;
mod queue;

use lila::{Game, Lila};
use queue::Queue;
pub use queue::{QueueFull, Ticket};

const MAX_PLIES: usize = 50;

#[derive(Parser, Clone)]
pub struct IndexerOpt {
    /// Base url for the indexer.
    #[arg(long = "lila", default_value = "https://lichess.org")]
    lila: String,
    /// Token of https://lichess.org/@/OpeningExplorer to speed up indexing.
    #[arg(long = "bearer", env = "EXPLORER_BEARER")]
    bearer: Option<String>,
    /// Number of parallel indexing tasks.
    #[arg(long = "indexers", default_value = "8")]
    indexers: usize,
}

#[derive(Clone)]
pub struct IndexerStub {
    queue: Arc<Queue<UserId>>,
    db: Arc<Database>,
}

impl IndexerStub {
    pub fn spawn(db: Arc<Database>, opt: IndexerOpt) -> (IndexerStub, Vec<JoinHandle<()>>) {
        let queue = Arc::new(Queue::with_capacity(2000));

        let mut join_handles = Vec::with_capacity(opt.indexers);
        for idx in 0..opt.indexers {
            join_handles.push(tokio::spawn(
                IndexerActor {
                    idx,
                    queue: Arc::clone(&queue),
                    db: Arc::clone(&db),
                    lila: Lila::new(opt.clone()),
                }
                .run(),
            ));
        }

        (IndexerStub { queue, db }, join_handles)
    }

    pub fn num_indexing(&self) -> usize {
        self.queue.len()
    }

    pub fn preceding_tickets(&self, ticket: &Ticket) -> u64 {
        self.queue.preceding_tickets(ticket)
    }

    pub async fn index_player(
        &self,
        player: UserId,
        semaphore: &Semaphore,
    ) -> Result<Ticket, QueueFull<UserId>> {
        if let Some(ticket) = self.queue.watch(&player) {
            return Ok(ticket);
        }

        let status = {
            let player = player.clone();
            let db = Arc::clone(&self.db);
            spawn_blocking(semaphore, move || {
                db.lichess()
                    .player_status(&player)
                    .expect("get player status")
                    .unwrap_or_default()
            })
            .await
        };

        if status.maybe_start_index_run().is_none() {
            return Ok(Ticket::new_completed()); // Do not reindex so soon!
        }

        self.queue.submit(player)
    }
}

struct IndexerActor {
    idx: usize,
    queue: Arc<Queue<UserId>>,
    db: Arc<Database>,
    lila: Lila,
}

impl IndexerActor {
    async fn run(self) {
        loop {
            let queue_item = self.queue.acquire().await;
            self.index_player(queue_item.task()).await;
        }
    }

    async fn index_player(&self, player: &UserId) {
        let mut status = {
            let db = Arc::clone(&self.db);
            let player = player.clone();
            task::spawn_blocking(move || {
                db.lichess()
                    .player_status(&player)
                    .expect("get player status")
                    .unwrap_or_default()
            })
            .await
            .expect("blocking get player status")
        };

        let index_run = match status.maybe_start_index_run() {
            Some(index_run) => index_run,
            None => return, // Do not reindex so soon!
        };

        let started_at = Instant::now();
        log::info!(
            "indexer {:02}: starting {} ({})",
            self.idx,
            player.as_lowercase_str(),
            index_run,
        );

        let mut games = match timeout(
            Duration::from_secs(60),
            self.lila.user_games(player, index_run.since()),
        )
        .await
        {
            Ok(Ok(games)) => games,
            Ok(Err(err)) if err.status() == Some(StatusCode::NOT_FOUND) => {
                log::warn!(
                    "indexer {:02}: did not find player {}",
                    self.idx,
                    player.as_lowercase_str()
                );
                return;
            }
            Ok(Err(err)) => {
                log::error!("indexer {:02}: request failed: {}", self.idx, err);
                sleep(Duration::from_secs(5)).await;
                return;
            }
            Err(timed_out) => {
                log::error!("indexer {:02}: request to lila: {}", self.idx, timed_out);
                return;
            }
        };

        let (tx_game, mut rx_game) = mpsc::channel(100);
        let idx = self.idx;
        tokio::spawn(async move {
            loop {
                let game = match timeout(Duration::from_secs(60), games.next()).await {
                    Ok(Some(Ok(game))) => game,
                    Ok(Some(Err(err))) => {
                        log::error!("indexer {:02}: {}", idx, err);
                        continue;
                    }
                    Ok(None) => break,
                    Err(timed_out) => {
                        log::error!("indexer {:02}: stream from lila: {}", idx, timed_out);
                        break;
                    }
                };
                if tx_game.send(game).await.is_err() {
                    log::error!("indexer {:02}: game receiver dropped", idx);
                    break;
                }
            }
        });

        let hash = ByColor::new_with(|color| KeyBuilder::player(player, color));

        let mut num_games = 0;
        while let Some(game) = rx_game.recv().await {
            task::block_in_place(|| {
                self.index_game(player, &hash, game, &mut status);

                num_games += 1;
                if num_games % 1024 == 0 {
                    self.db
                        .lichess()
                        .put_player_status(player, &status)
                        .expect("put player status");

                    log::info!(
                        "indexer {:02}: indexed {} games for {} ...",
                        self.idx,
                        num_games,
                        player.as_lowercase_str()
                    );
                }
            });
        }

        status.finish_index_run(index_run);
        task::block_in_place(|| {
            self.db
                .lichess()
                .put_player_status(player, &status)
                .expect("put player status");
        });

        let elapsed = started_at.elapsed();

        if num_games > 0 {
            log::info!(
                "indexer {:02}: finished {} games for {} in {:.3?} ({:.3?}/game, {:.1} games/s)",
                self.idx,
                num_games,
                player.as_lowercase_str(),
                elapsed,
                elapsed / num_games,
                f64::from(num_games) / elapsed.as_secs_f64()
            );
        } else {
            log::info!(
                "indexer {:02}: no new games for {}",
                self.idx,
                player.as_lowercase_str()
            );
        }
    }

    fn index_game(
        &self,
        player: &UserId,
        hash: &ByColor<KeyBuilder>,
        game: Game,
        status: &mut PlayerStatus,
    ) {
        status.latest_created_at = game.created_at;

        if game.status.is_ongoing() {
            if status.revisit_ongoing_created_at.is_none() {
                log::info!(
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

        if game
            .players
            .iter()
            .any(|p| p.user.is_none() || p.rating.is_none())
        {
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
                    player.as_lowercase_str(),
                    game.id
                );
                return;
            }
        };

        // Skip game if already indexed from this side. This cannot race with
        // writes, because all writes for the same player are sequenced by
        // this actor. So making a transaction is not required.
        let lichess_db = self.db.lichess();
        if lichess_db
            .game(game.id)
            .expect("get game info")
            .map_or(false, |info| *info.indexed_player.get(color))
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
        let mut pos = match game.initial_fen {
            Some(fen) => {
                match VariantPosition::from_setup(
                    game.variant,
                    fen.into_setup(),
                    CastlingMode::Chess960,
                ) {
                    Ok(pos) => pos,
                    Err(err) => {
                        log::warn!("indexer {:02}: not indexing {}: {}", self.idx, game.id, err);
                        return;
                    }
                }
            }
            None => VariantPosition::new(game.variant),
        };
        let opponent_rating = match game.players.get(!color).rating {
            Some(rating) => rating,
            None => {
                log::warn!(
                    "indexer {:02}: skipping {} without opponent rating",
                    self.idx,
                    game.id
                );
                return;
            }
        };

        // Build an intermediate table to remove loops (due to repetitions).
        let mut without_loops: IntMap<Zobrist128, Uci> =
            HashMap::with_capacity_and_hasher(game.moves.len(), Default::default());

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
            without_loops.insert(pos.zobrist_hash(shakmaty::EnPassantMode::Legal), uci);

            pos.play_unchecked(&m);
        }

        // Write to database. All writes regarding this game are batched and
        // atomically committed, so the database will always be in a consistent
        // state.
        let mut batch = lichess_db.batch();

        batch.merge_game(
            game.id,
            LichessGame {
                outcome,
                speed: game.speed,
                mode: Mode::from_rated(game.rated),
                month,
                players: game.players.map(|p| GamePlayer {
                    name: p.user.map_or(String::new(), |u| u.name.to_string()),
                    rating: p.rating.unwrap_or_default(),
                }),
                indexed_player: ByColor::new_with(|c| color == c),
                indexed_lichess: false,
            },
        );

        for (zobrist, uci) in without_loops {
            batch.merge_player(
                hash.get(color)
                    .with_zobrist(game.variant, zobrist)
                    .with_month(month),
                PlayerEntry::new_single(
                    uci.clone(),
                    game.speed,
                    Mode::from_rated(game.rated),
                    game.id,
                    outcome,
                    opponent_rating,
                ),
            );
        }

        batch.commit().expect("atomically commit game and moves");
    }
}
