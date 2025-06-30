use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nohash_hasher::IntMap;
use shakmaty::{
    Chess, Color, EnPassantMode, Outcome, Position, uci::UciMove, variant::Variant,
    zobrist::ZobristHash,
};

use crate::{
    api::Error,
    db::Database,
    model::{KeyBuilder, LaxDate, MastersEntry, MastersGameWithId},
    util::midpoint,
    zobrist::StableZobrist128,
};

#[derive(Clone)]
pub struct MastersImporter {
    db: Arc<Database>,
    mutex: Arc<Mutex<()>>,
}

impl MastersImporter {
    pub fn new(db: Arc<Database>) -> MastersImporter {
        MastersImporter {
            db,
            mutex: Arc::new(Mutex::new(())),
        }
    }

    pub fn import(&self, body: MastersGameWithId) -> Result<(), Error> {
        let avg_rating = midpoint(
            body.game.players.white.rating,
            body.game.players.black.rating,
        );
        if avg_rating < 2200 {
            return Err(Error::RejectedRating {
                id: body.id,
                rating: avg_rating,
            });
        }

        if body.game.date.is_definitely_after(LaxDate::tomorrow()) {
            return Err(Error::RejectedDate {
                id: body.id,
                date: body.game.date,
            });
        }

        let _guard = self.mutex.lock().expect("lock masters db");
        let masters_db = self.db.masters();

        if masters_db
            .has_game(body.id)
            .expect("check for masters game")
        {
            return Err(Error::DuplicateGame { id: body.id });
        }

        let mut without_loops: IntMap<StableZobrist128, (UciMove, Color)> =
            HashMap::with_capacity_and_hasher(body.game.moves.len(), Default::default());
        let mut pos = Chess::default();
        let mut final_key = None;
        for uci in &body.game.moves {
            let key = pos.zobrist_hash(EnPassantMode::Legal);
            final_key = Some(key);
            let m = uci.to_move(&pos)?;
            without_loops.insert(key, (UciMove::from_chess960(m), pos.turn()));
            pos.play_unchecked(m);
        }

        if let Some(final_key) = final_key {
            if masters_db
                .has(
                    KeyBuilder::masters()
                        .with_zobrist(Variant::Chess, final_key)
                        .with_year(body.game.date.year()),
                )
                .expect("check for masters entry")
            {
                return Err(Error::DuplicateGame { id: body.id });
            }
        }

        let mut batch = masters_db.batch();
        batch.put_game(body.id, &body.game);
        for (key, (uci, turn)) in without_loops {
            batch.merge(
                KeyBuilder::masters()
                    .with_zobrist(Variant::Chess, key)
                    .with_year(body.game.date.year()),
                MastersEntry::new_single(
                    uci,
                    body.id,
                    Outcome::from_winner(body.game.winner),
                    body.game.players.get(turn).rating,
                    body.game.players.get(!turn).rating,
                ),
            );
        }

        batch.commit().expect("commit masters game");
        Ok(())
    }
}
