use std::sync::Arc;

use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};
use shakmaty::{
    fen::Fen,
    san::San,
    uci::Uci,
    variant::{Variant, VariantPosition},
    zobrist::Zobrist,
    ByColor, CastlingMode, Chess, Color, Outcome, Position, Setup,
};
use tokio::sync::Mutex;

use crate::{
    api::{Error, LilaVariant},
    db::Database,
    model::{
        GameId, GameInfo, GameInfoPlayer, Key, KeyBuilder, LaxDate, LichessEntry, MasterEntry,
        MasterGameWithId, Mode, Speed,
    },
    util::ByColorDef,
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
        if body.game.players.white.rating / 2 + body.game.players.black.rating / 2 < 2200 {
            return Err(Error::RejectedImport(body.id));
        }

        let _guard = self.mutex.lock();
        let queryable = self.db.queryable();
        if queryable
            .has_master_game(body.id)
            .expect("check for master game")
        {
            return Err(Error::DuplicateGame(body.id));
        }

        let mut without_loops: FxHashMap<Key, (Uci, Color)> =
            FxHashMap::with_capacity_and_hasher(body.game.moves.len(), Default::default());
        let mut pos: Zobrist<Chess, u128> = Zobrist::default();
        let mut final_key = None;
        for uci in &body.game.moves {
            let key = KeyBuilder::master()
                .with_zobrist(Variant::Chess, pos.zobrist_hash())
                .with_year(body.game.date.year());
            final_key = Some(key.clone());
            let m = uci.to_move(&pos)?;
            without_loops.insert(key, (Uci::from_chess960(&m), pos.turn()));
            pos.play_unchecked(&m);
        }

        if let Some(final_key) = final_key {
            if queryable
                .has_master(final_key)
                .expect("check for master entry")
            {
                return Err(Error::DuplicateGame(body.id));
            }
        }

        let mut batch = queryable.batch();
        batch.put_master_game(body.id, &body.game);
        for (key, (uci, turn)) in without_loops {
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

        batch.write().expect("commit master game");
        Ok(())
    }
}

#[serde_as]
#[derive(Deserialize)]
pub struct LichessGame {
    variant: LilaVariant,
    speed: Speed,
    #[serde_as(as = "Option<DisplayFromStr>")]
    fen: Option<Fen>,
    #[serde_as(as = "DisplayFromStr")]
    id: GameId,
    #[serde_as(as = "DisplayFromStr")]
    date: LaxDate,
    #[serde(with = "ByColorDef")]
    players: ByColor<GameInfoPlayer>,
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

    pub async fn import(&self, game: LichessGame) -> Result<(), Error> {
        let _guard = self.mutex.lock();

        let queryable = self.db.queryable();

        if queryable
            .get_game_info(game.id)
            .expect("get game info")
            .map_or(false, |info| info.indexed_lichess)
        {
            log::warn!("lichess game {} already imported", game.id);
            return Ok(());
        }

        let month = match game.date.month() {
            Some(month) => month,
            None => {
                log::error!("lichess game {} missing month", game.id);
                return Err(Error::RejectedImport(game.id));
            }
        };
        let outcome = Outcome::from_winner(game.winner);
        let variant = Variant::from(game.variant);

        let mut pos: Zobrist<_, u128> = Zobrist::new(match game.fen {
            Some(fen) => {
                VariantPosition::from_setup(variant, &Fen::from(fen), CastlingMode::Chess960)?
            }
            None => VariantPosition::new(variant),
        });

        let mut without_loops: FxHashMap<Key, (Uci, Color)> =
            FxHashMap::with_capacity_and_hasher(game.moves.len(), Default::default());
        for san in game.moves {
            let m = san.to_move(&pos)?;
            without_loops.insert(
                KeyBuilder::lichess()
                    .with_zobrist(variant, pos.zobrist_hash())
                    .with_month(month),
                (Uci::from_chess960(&m), pos.turn()),
            );
            pos.play_unchecked(&m);
        }

        let mut batch = queryable.batch();
        batch.merge_game_info(
            game.id,
            GameInfo {
                mode: Mode::Rated,
                indexed_personal: Default::default(),
                indexed_lichess: true,
                outcome,
                players: game.players.clone(),
                month,
                speed: game.speed,
            },
        );
        for (key, (uci, turn)) in without_loops {
            batch.merge_lichess(
                key,
                LichessEntry::new_single(
                    uci,
                    game.speed,
                    game.id,
                    outcome,
                    game.players.by_color(turn).rating,
                    game.players.by_color(!turn).rating,
                ),
            );
        }

        batch.write().expect("commit lichess game");
        Ok(())
    }
}
