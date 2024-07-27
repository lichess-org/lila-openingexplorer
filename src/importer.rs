use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nohash_hasher::IntMap;
use serde::Deserialize;
use serde_with::{
    formats::SpaceSeparator, serde_as, DefaultOnNull, DisplayFromStr, StringWithSeparator,
};
use shakmaty::{
    fen::Fen,
    san::San,
    uci::UciMove,
    variant::{Variant, VariantPosition},
    zobrist::ZobristHash,
    ByColor, CastlingMode, Chess, Color, EnPassantMode, Outcome, Position,
};

use crate::{
    api::Error,
    db::Database,
    model::{
        GameId, GamePlayer, KeyBuilder, LaxDate, LichessEntry, LichessGame, MastersEntry,
        MastersGameWithId, Mode, Speed, Year,
    },
    util::{midpoint, ByColorDef},
    zobrist::StableZobrist128,
};

const MAX_PLIES: usize = 50;

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

        let year = body.game.date.year();
        if Year::max_masters() < year {
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
            without_loops.insert(key, (UciMove::from_chess960(&m), pos.turn()));
            pos.play_unchecked(&m);
        }

        if let Some(final_key) = final_key {
            if masters_db
                .has(
                    KeyBuilder::masters()
                        .with_zobrist(Variant::Chess, final_key)
                        .with_year(year),
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
                    .with_year(year),
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

#[serde_as]
#[derive(Deserialize)]
pub struct LichessGameImport {
    #[serde_as(as = "DefaultOnNull<DisplayFromStr>")]
    variant: Variant,
    speed: Speed,
    #[serde_as(as = "Option<DisplayFromStr>")]
    fen: Option<Fen>,
    #[serde_as(as = "DisplayFromStr")]
    id: GameId,
    #[serde_as(as = "DisplayFromStr")]
    date: LaxDate,
    #[serde(flatten, with = "ByColorDef")]
    players: ByColor<GamePlayer>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    winner: Option<Color>,
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, San>")]
    moves: Vec<San>,
}

#[derive(Clone)]
pub struct LichessImporter {
    db: Arc<Database>,
    mutex: Arc<Mutex<()>>,
}

impl LichessImporter {
    pub fn new(db: Arc<Database>) -> LichessImporter {
        LichessImporter {
            db,
            mutex: Arc::new(Mutex::new(())),
        }
    }

    pub fn import_many(&self, games: Vec<LichessGameImport>) -> Result<(), Error> {
        for game in games {
            self.import(game)?;
        }
        Ok(())
    }

    fn import(&self, game: LichessGameImport) -> Result<(), Error> {
        let _guard = self.mutex.lock().expect("lock lichess db");

        let lichess_db = self.db.lichess();
        if lichess_db
            .game(game.id)
            .expect("get game info")
            .map_or(false, |info| info.indexed_lichess)
        {
            log::debug!("lichess game {} already imported", game.id);
            return Ok(());
        }

        let month = match game.date.month() {
            Some(month) => month,
            None => {
                log::error!("lichess game {} missing month", game.id);
                return Err(Error::RejectedDate {
                    id: game.id,
                    date: game.date,
                });
            }
        };
        let outcome = Outcome::from_winner(game.winner);

        let mut pos = match game.fen {
            Some(fen) => {
                VariantPosition::from_setup(game.variant, fen.into_setup(), CastlingMode::Chess960)?
            }
            None => VariantPosition::new(game.variant),
        };

        let mut without_loops: IntMap<StableZobrist128, (UciMove, Color)> =
            HashMap::with_capacity_and_hasher(game.moves.len(), Default::default());
        for san in game.moves.into_iter().take(MAX_PLIES) {
            let m = san.to_move(&pos)?;
            without_loops.insert(
                pos.zobrist_hash(EnPassantMode::Legal),
                (UciMove::from_chess960(&m), pos.turn()),
            );
            pos.play_unchecked(&m);
        }

        let mut batch = lichess_db.batch();
        for (key, (uci, turn)) in without_loops {
            batch.merge_lichess(
                KeyBuilder::lichess()
                    .with_zobrist(game.variant, key)
                    .with_month(month),
                LichessEntry::new_single(
                    uci,
                    game.speed,
                    game.id,
                    outcome,
                    game.players.get(turn).rating,
                    game.players.get(!turn).rating,
                ),
            );
        }
        batch.merge_game(
            game.id,
            LichessGame {
                mode: Mode::Rated,
                indexed_player: Default::default(),
                indexed_lichess: true,
                outcome,
                players: game.players,
                month,
                speed: game.speed,
            },
        );

        batch.commit().expect("commit lichess game");
        Ok(())
    }
}
