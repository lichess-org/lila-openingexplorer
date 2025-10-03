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
    ByColor, CastlingMode, KnownOutcome, Position, uci::UciMove, variant::VariantPosition,
    zobrist::ZobristHash,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task,
    task::JoinSet,
    time::{sleep, timeout},
};

use crate::{
    db::Database,
    indexer::{Queue, QueueFull, Ticket},
    lila::{Game, Lila, LilaOpt},
    model::{GamePlayer, KeyBuilder, LichessGame, Mode, Month, PlayerEntry, PlayerStatus, UserId},
    util::spawn_blocking,
    zobrist::StableZobrist128,
};

const MAX_PLIES: usize = 50;

#[derive(Parser, Clone)]
pub struct PlayerIndexerOpt {
    /// Number of parallel indexing tasks.
    #[arg(long = "indexers", default_value = "8")]
    indexers: usize,
}

#[derive(Clone)]
pub struct PlayerIndexerStub {
    queue: Arc<Queue<UserId>>,
    db: Arc<Database>,
}

impl PlayerIndexerStub {
    pub fn spawn(
        join_set: &mut JoinSet<()>,
        db: Arc<Database>,
        opt: PlayerIndexerOpt,
        lila_opt: LilaOpt,
    ) -> PlayerIndexerStub {
        let queue = Arc::new(Queue::with_capacity(2000));

        for idx in 0..opt.indexers {
            join_set.spawn(
                PlayerIndexerActor {
                    idx,
                    queue: Arc::clone(&queue),
                    db: Arc::clone(&db),
                    lila: Lila::new(lila_opt.clone()),
                }
                .run(),
            );
        }

        PlayerIndexerStub { queue, db }
    }

    pub fn num_indexing(&self) -> usize {
        self.queue.estimate_len()
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

struct PlayerIndexerActor {
    idx: usize,
    queue: Arc<Queue<UserId>>,
    db: Arc<Database>,
    lila: Lila,
}

impl PlayerIndexerActor {
    async fn run(self) {
        loop {
            let queue_item = self.queue.acquire().await;
            self.index_player(queue_item.task()).await;
        }
    }

    async fn feed_games(&self, player: &UserId, since: u64, tx: mpsc::Sender<Game>) {
        let mut games =
            match timeout(Duration::from_secs(60), self.lila.user_games(player, since)).await {
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

        loop {
            let game = match timeout(Duration::from_secs(60), games.next()).await {
                Ok(Some(Ok(game))) => game,
                Ok(Some(Err(err))) => {
                    log::error!("indexer {:02}: {}", self.idx, err);
                    continue;
                }
                Ok(None) => break,
                Err(timed_out) => {
                    log::error!("indexer {:02}: stream from lila: {}", self.idx, timed_out);
                    break;
                }
            };

            if tx.send(game).await.is_err() {
                log::error!("indexer {:02}: game receiver dropped", self.idx);
                break;
            }
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
            .expect("join get player status")
        };

        let index_run = match status.maybe_start_index_run() {
            Some(index_run) => index_run,
            None => return, // Do not reindex so soon!
        };

        let index_run_since = index_run.since();
        let (tx_game, mut rx_game) = mpsc::channel(100);

        let join_handle = {
            let idx = self.idx;
            let db = Arc::clone(&self.db);
            let player = player.clone();

            task::spawn_blocking(move || {
                let started_at = Instant::now();
                log::info!(
                    "indexer {:02}: starting {} ({})",
                    idx,
                    player.as_lowercase_str(),
                    index_run,
                );

                let hash = ByColor::new_with(|color| KeyBuilder::player(&player, color));

                let mut num_games = 0;
                while let Some(game) = rx_game.blocking_recv() {
                    PlayerIndexerActor::index_game(idx, &db, &player, &hash, game, &mut status);
                    num_games += 1;

                    if num_games % 1024 == 0 {
                        db.lichess()
                            .put_player_status(&player, &status)
                            .expect("put player status");

                        log::info!(
                            "indexer {:02}: indexed {} games for {} ...",
                            idx,
                            num_games,
                            player.as_lowercase_str()
                        );
                    }
                }

                status.finish_index_run(index_run);
                db.lichess()
                    .put_player_status(&player, &status)
                    .expect("put player status");

                let elapsed = started_at.elapsed();

                if num_games > 0 {
                    log::info!(
                        "indexer {:02}: finished {} games for {} in {:.3?} ({:.3?}/game, {:.1} games/s)",
                        idx,
                        num_games,
                        player.as_lowercase_str(),
                        elapsed,
                        elapsed / num_games,
                        f64::from(num_games) / elapsed.as_secs_f64()
                    );
                } else {
                    log::info!(
                        "indexer {:02}: no new games for {}",
                        idx,
                        player.as_lowercase_str()
                    );
                }
            })
        };

        self.feed_games(player, index_run_since, tx_game).await;
        join_handle.await.expect("join index player");
    }

    fn index_game(
        idx: usize,
        db: &Database,
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
                    idx,
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
            .find(|p| p.user.as_ref().is_some_and(|user| user.name == *player))
        {
            Some(color) => color,
            None => {
                log::error!(
                    "indexer {:02}: {} did not play in {}",
                    idx,
                    player.as_lowercase_str(),
                    game.id
                );
                return;
            }
        };

        // Skip game if already indexed from this side. This cannot race with
        // writes, because all writes for the same player are sequenced by
        // this actor. So making a transaction is not required.
        let lichess_db = db.lichess();
        if lichess_db
            .game(game.id)
            .expect("get game info")
            .is_some_and(|info| *info.indexed_player.get(color))
        {
            log::debug!("indexer {:02}: {}/{} already indexed", idx, game.id, color);
            return;
        }

        // Prepare basic information and setup initial position.
        let month = Month::from_time_saturating(game.last_move_at);
        let outcome = KnownOutcome::from_winner(game.winner);
        let mut pos = match game.initial_fen {
            Some(fen) => {
                match VariantPosition::from_setup(
                    game.variant,
                    fen.into_setup(),
                    CastlingMode::Chess960,
                ) {
                    Ok(pos) => pos,
                    Err(err) => {
                        log::warn!("indexer {:02}: not indexing {}: {}", idx, game.id, err);
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
                    idx,
                    game.id
                );
                return;
            }
        };

        // Build an intermediate table to remove loops (due to repetitions).
        let mut without_loops: IntMap<StableZobrist128, UciMove> =
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
                        idx,
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

            pos.play_unchecked(m);
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
                    uci,
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
