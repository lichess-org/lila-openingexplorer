use crate::api::Error;
use crate::db::Database;
use crate::model::{Mode, PersonalEntry, PersonalKeyBuilder, UserName};
use crate::util::NevermindExt as _;
use futures_util::StreamExt;
use rustc_hash::FxHashMap;
use shakmaty::{
    uci::Uci, variant::VariantPosition, zobrist::Zobrist, ByColor, CastlingMode, Color, Outcome,
    Position,
};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::{
    sync::{
        mpsc::{self, error::SendTimeoutError},
        oneshot,
    },
    task::JoinHandle,
    time::timeout,
};

mod lila;

use lila::Lila;

#[derive(Clone)]
pub struct IndexerStub {
    tx: mpsc::Sender<IndexerMessage>,
}

impl IndexerStub {
    pub fn spawn(db: Arc<Database>) -> (IndexerStub, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(2);
        (
            IndexerStub { tx },
            tokio::spawn(
                IndexerActor {
                    rx,
                    db,
                    lila: Lila::new(),
                }
                .run(),
            ),
        )
    }

    pub async fn index_player(&self, player: UserName) -> Result<IndexerStatus, Error> {
        let (req, res) = oneshot::channel();

        self.tx
            .send_timeout(
                IndexerMessage::IndexPlayer {
                    player,
                    callback: req,
                },
                Duration::from_secs(2),
            )
            .await
            .map_err(|err| match err {
                SendTimeoutError::Timeout(_) => Error::IndexerQueueFull,
                SendTimeoutError::Closed(_) => panic!("indexer died"),
            })?;

        match timeout(Duration::from_secs(7), res).await {
            Ok(res) => match res.expect("indexer alive") {
                Ok(()) => Ok(IndexerStatus::Completed),
                Err(err) => Err(err),
            },
            Err(_) => return Ok(IndexerStatus::Ongoing),
        }
    }
}

struct IndexerActor {
    rx: mpsc::Receiver<IndexerMessage>,
    db: Arc<Database>,
    lila: Lila,
}

impl IndexerActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                IndexerMessage::IndexPlayer { callback, player } => {
                    callback
                        .send(self.index_player(player).await)
                        .nevermind("requester gone away");
                }
            }
        }
    }

    async fn index_player(&self, player: UserName) -> Result<(), Error> {
        log::info!("indexing {}", player);

        let mut games = self.lila.user_games(player).await?;
        while let Some(game) = games.next().await {
            let game = game?;

            if game.status.is_unindexable() {
                continue;
            }

            let outcome = Outcome::from_winner(game.winner);

            let hash = ByColor::new_with(|color| {
                game.user_name(color)
                    .map(|u| PersonalKeyBuilder::with_user_pov(&u.to_owned().into(), color))
            });

            let pos = match game.initial_fen {
                Some(fen) => {
                    VariantPosition::from_setup(game.variant.into(), &fen, CastlingMode::Chess960)
                }
                None => Ok(VariantPosition::new(game.variant.into())),
            };

            let pos = match pos {
                Ok(pos) => pos,
                Err(err) => {
                    log::error!("indexing {}: {}", game.id, err);
                    continue;
                }
            };

            let mut pos: Zobrist<_, u128> = Zobrist::new(pos);

            // Build an intermediate table to remove loops (due to repetitions).
            let mut table: FxHashMap<u128, Uci> =
                FxHashMap::with_capacity_and_hasher(game.moves.len(), Default::default());

            for san in game.moves {
                let m = match san.to_move(&pos) {
                    Ok(m) => m,
                    Err(err) => {
                        log::error!("indexing {}: {}", game.id, err);
                        continue;
                    }
                };

                let uci = m.to_uci(CastlingMode::Chess960);
                table.insert(pos.zobrist_hash(), uci);

                pos.play_unchecked(&m);
            }

            let queryable = self.db.queryable();

            for (zobrist, uci) in table {
                for color in [Color::White, Color::Black] {
                    if let Some(builder) = hash.by_color(color) {
                        let entry = PersonalEntry::new_single(
                            uci.clone(),
                            game.speed,
                            Mode::from_rated(game.rated),
                            game.id,
                            outcome,
                        );

                        let mut buf = Cursor::new(Vec::new());
                        entry.write(&mut buf).expect("serialize personal entry");

                        queryable
                            .db
                            .put_cf(
                                queryable.cf_personal,
                                builder.with_zobrist(zobrist).prefix(),
                                buf.into_inner(),
                            )
                            .expect("merge cf personal");
                    }
                }
            }
        }
        Ok(())
    }
}

enum IndexerMessage {
    IndexPlayer {
        player: UserName,
        callback: oneshot::Sender<Result<(), Error>>,
    },
}

pub enum IndexerStatus {
    Ongoing,
    Completed,
    Failed,
}
