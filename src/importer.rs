use std::sync::Arc;

use rustc_hash::FxHashMap;
use shakmaty::{uci::Uci, zobrist::Zobrist, Chess, Position};
use tokio::sync::Mutex;

use crate::{api::Error, db::Database, model::MasterGameWithId};

#[derive(Clone)]
pub struct MasterImporter {
    db: Arc<Database>,
    mutex: Arc<Mutex<()>>,
}

impl MasterImporter {
    pub fn new(db: Arc<Database>) -> MasterImporter {
        MasterImporter {
            db,
            mutex: Arc::new(Mutex::new(())),
        }
    }

    pub async fn import(&self, body: MasterGameWithId) -> Result<(), Error> {
        let _guard = self.mutex.lock();

        let mut without_loops: FxHashMap<u128, Uci> =
            FxHashMap::with_capacity_and_hasher(body.game.moves.len(), Default::default());
        let mut pos: Zobrist<Chess, u128> = Zobrist::default();
        for uci in &body.game.moves {
            let m = uci.to_move(&pos)?;
            without_loops.insert(pos.zobrist_hash(), Uci::from_chess960(&m));
            pos.play_unchecked(&m);
        }

        let queryable = self.db.queryable();
        let mut batch = queryable.batch();
        batch.put_master_game(body.id, &body.game);
        for (zobrist, uci) in without_loops {
            todo!();
            // batch.merge_master();
        }
        batch.write().expect("commit master game");

        Ok(())
    }
}
