use crate::api::UserName;
use crate::db::Database;
use crate::util::NevermindExt as _;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub struct IndexerActor {
    pub rx: mpsc::Receiver<IndexerMessage>,
    pub db: Arc<Database>,
}

impl IndexerActor {
    pub async fn run(mut self) {
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

    async fn index_player(&self, player: UserName) {}
}

pub enum IndexerMessage {
    IndexPlayer {
        player: UserName,
        callback: oneshot::Sender<()>,
    },
}
