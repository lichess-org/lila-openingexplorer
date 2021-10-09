use crate::api::{Error, UserName};
use crate::db::Database;
use crate::util::NevermindExt as _;
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
            tokio::spawn(IndexerActor {
                rx,
                db,
                lila: Lila::new(),
            }.run()),
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
                    callback.send(self.index_player(player).await).nevermind("requester gone away");
                }
            }
        }
    }

    async fn index_player(&self, player: UserName) -> Result<(), Error> {
        let games = self.lila.user_games(player);
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
