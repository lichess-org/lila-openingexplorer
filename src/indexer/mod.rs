use crate::api::{Error, UserName};
use crate::db::Database;
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

mod actor;
mod lila;

use actor::{IndexerActor, IndexerMessage};

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
                SendTimeoutError::Timeout(_) => Error::IndexerQueueFull,
                SendTimeoutError::Closed(_) => panic!("indexer died"),
            })?;

        Ok(match timeout(Duration::from_secs(7), res).await {
            Ok(Ok(_)) => IndexerStatus::Completed,
            Ok(Err(_)) => IndexerStatus::Failed,
            Err(_) => IndexerStatus::Ongoing,
        })
    }
}

pub enum IndexerStatus {
    Ongoing,
    Completed,
    Failed,
}
