use std::sync::Arc;

use rustc_hash::FxHashMap;
use shakmaty::{
    uci::Uci, variant::Variant, zobrist::Zobrist, Chess, Color, Outcome, Position, Setup,
};
use tokio::sync::Mutex;

use crate::{
    api::Error,
    db::Database,
    model::{KeyBuilder, MasterEntry, MasterGameWithId},
};

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

        let queryable = self.db.queryable();
        if queryable.has_master_game(body.id).expect("check for master game") {
            return Err(Error::DuplicateGame(body.id));
        }

        let mut without_loops: FxHashMap<u128, (Uci, Color)> =
            FxHashMap::with_capacity_and_hasher(body.game.moves.len(), Default::default());
        let mut pos: Zobrist<Chess, u128> = Zobrist::default();
        for uci in &body.game.moves {
            let m = uci.to_move(&pos)?;
            without_loops.insert(pos.zobrist_hash(), (Uci::from_chess960(&m), pos.turn()));
            pos.play_unchecked(&m);
        }

        let mut batch = queryable.batch();
        let mut final_key = None;
        batch.put_master_game(body.id, &body.game);
        for (zobrist, (uci, turn)) in without_loops {
            let key = KeyBuilder::master().with_zobrist(Variant::Chess, zobrist).with_month(todo!());
            final_key = Some(key);
            batch.merge_master(
                key,
                MasterEntry::new_single(
                    uci,
                    body.id,
                    Outcome::from_winner(body.game.winner),
                    body.game.players.by_color(turn).rating,
                    body.game.players.by_color(!turn).rating,
                ),
            );
        }

        if let Some(final_key) = final_key {
            if queryable.has_master(final_key).expect("check for master entry") {
                return Err(Error::DuplicateGame(body.id));
            }
        }

        batch.write().expect("commit master game");
        Ok(())
    }
}
