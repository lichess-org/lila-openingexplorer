use std::sync::Arc;
use tokio::{sync::{mpsc, oneshot}, task::JoinHandle};
use crate::db::Database;

pub struct IndexerStub {
    tx: mpsc::Sender<IndexerMessage>,
}

impl IndexerStub {
    pub fn spawn(db: Arc<Database>) -> (IndexerStub, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(100);
        (IndexerStub { tx }, tokio::spawn(IndexerActor { rx, db }.run()))
    }

    pub fn index_player(&self) {
        self.tx.send(); // XXX
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
                IndexerMessage::IndexPlayer { callback } => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    callback.send(());
                }
            }
        }
    }
}

enum IndexerMessage {
    IndexPlayer {
        callback: oneshot::Sender<()>,
    }
}
