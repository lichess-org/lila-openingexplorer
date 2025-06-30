use std::time::Duration;

use nohash_hasher::IntMap;
use serde::{Deserialize, Serialize};
use shakmaty::{
    Chess, EnPassantMode, Position,
    san::San,
    uci::UciMove,
    variant::{Variant, VariantPosition},
    zobrist::{Zobrist64, ZobristHash},
};

use crate::api::Error;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    eco: String,
    name: String,
}

#[derive(Deserialize)]
struct OpeningRecord {
    eco: String,
    name: String,
    pgn: String,
}

#[derive(Default)]
pub struct Openings {
    data: IntMap<Zobrist64, Opening>,
}

impl Openings {
    pub fn new() -> Openings {
        Openings::default()
    }

    pub async fn download() -> Result<Openings, Error> {
        let mut openings = Openings::new();
        let client = reqwest::Client::builder()
            .user_agent("lila-openingexplorer")
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");
        for part in ["a", "b", "c", "d", "e"] {
            let tsv = client
                .get(format!(
                    "https://raw.githubusercontent.com/lichess-org/chess-openings/master/{part}.tsv"
                ))
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            openings.load_tsv(&tsv)?;
        }
        Ok(openings)
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn load_tsv(&mut self, tsv: &str) -> Result<(), Error> {
        let mut tsv = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .from_reader(tsv.as_bytes());

        for record in tsv.deserialize() {
            let record: OpeningRecord = record?;

            let mut pos = Chess::default();
            for token in record.pgn.split(' ') {
                if let Ok(san) = token.parse::<San>() {
                    pos.play_unchecked(san.to_move(&pos)?);
                }
            }

            if self
                .data
                .insert(
                    pos.zobrist_hash(EnPassantMode::Legal),
                    Opening {
                        eco: record.eco,
                        name: record.name,
                    },
                )
                .is_some()
            {
                return Err(Error::DuplicateOpening);
            }
        }

        Ok(())
    }

    pub fn classify_and_play(
        &self,
        root: &mut VariantPosition,
        play: Vec<UciMove>,
    ) -> Result<Option<Opening>, Error> {
        let mut opening = self.classify_exact(root);

        for uci in play {
            let m = uci.to_move(root)?;
            root.play_unchecked(m);

            opening = self.classify_exact(root).or(opening);
        }

        Ok(opening.cloned())
    }

    pub fn classify_exact(&self, pos: &VariantPosition) -> Option<&Opening> {
        if opening_sensible(pos.variant()) {
            self.data.get(&pos.zobrist_hash(EnPassantMode::Legal))
        } else {
            None
        }
    }
}

fn opening_sensible(variant: Variant) -> bool {
    matches!(
        variant,
        Variant::Chess | Variant::Crazyhouse | Variant::ThreeCheck | Variant::KingOfTheHill
    )
}
