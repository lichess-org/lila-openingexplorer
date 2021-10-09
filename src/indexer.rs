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

#[derive(Clone)]
pub struct IndexerStub {
    tx: mpsc::Sender<IndexerMessage>,
}

impl IndexerStub {
    pub fn spawn(db: Arc<Database>) -> (IndexerStub, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(2);
        (
            IndexerStub { tx },
            tokio::spawn(IndexerActor { rx, db }.run()),
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
                SendTimeoutError::Timeout(_) => Error::IndexerTooBusy,
                SendTimeoutError::Closed(_) => panic!("indexer died"),
            })?;

        Ok(match timeout(Duration::from_secs(7), res).await {
            Ok(Ok(res)) => IndexerStatus::Completed,
            Ok(Err(_)) => IndexerStatus::Failed,
            Err(_) => IndexerStatus::Ongoing,
        })
    }
}

struct IndexerActor {
    rx: mpsc::Receiver<IndexerMessage>,
    db: Arc<Database>,
}

impl IndexerActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                IndexerMessage::IndexPlayer { callback, player } => {
                    dbg!(player);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    callback.send(()).nevermind("user no longer waiting");
                }
            }
        }
    }

    async fn index_player(&self, player: UserName) {
    }
}

enum IndexerMessage {
    IndexPlayer {
        player: UserName,
        callback: oneshot::Sender<()>,
    },
}

pub enum IndexerStatus {
    Ongoing,
    Completed,
    Failed,
}
