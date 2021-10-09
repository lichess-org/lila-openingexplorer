use std::sync::Arc;
use tokio::{sync::{mpsc, oneshot}, task::JoinHandle};
use crate::db::Database;
use crate::api::UserName;

pub struct IndexerStub {
    tx: mpsc::Sender<IndexerMessage>,
}

impl IndexerStub {
    pub fn spawn(db: Arc<Database>) -> (IndexerStub, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(100);
        (IndexerStub { tx }, tokio::spawn(IndexerActor { rx, db }.run()))
    }

    /* XXX pub async fn index_player(&self) -> Result<(), ()> {
        let (req, res) = oneshot::channel();
        self.tx.send(IndexerMessage::IndexPlayer {
            callback: req
        }).expect("indexer actor alive");
        res.await;
    } */
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
                    callback.send(());
                }
            }
        }
    }
}

enum IndexerMessage {
    IndexPlayer {
        player: UserName,
        callback: oneshot::Sender<()>,
    }
}
