use std::{collections::HashMap, sync::Arc};

use nohash_hasher::IntMap;
use serde::{Deserialize, Serialize};
use shakmaty::{
    san::San,
    uci::Uci,
    variant::{Variant, VariantPosition},
    zobrist::{Zobrist64, ZobristHash},
    Chess, EnPassantMode, Position,
};

use crate::api::Error;

#[derive(Serialize, Clone, Debug)]
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

const TSV_DATA: [&str; 5] = [
    include_str!("../chess-openings/a.tsv"),
    include_str!("../chess-openings/b.tsv"),
    include_str!("../chess-openings/c.tsv"),
    include_str!("../chess-openings/d.tsv"),
    include_str!("../chess-openings/e.tsv"),
];

pub struct Openings {
    data: IntMap<Zobrist64, Opening>,
}

impl Default for Openings {
    fn default() -> Openings {
        let mut openings = Openings::empty();
        for tsv in TSV_DATA {
            openings.load_tsv(tsv).expect("valid opening tsv");
        }
        openings
    }
}

impl Openings {
    pub fn new() -> Openings {
        Openings::default()
    }

    pub fn empty() -> Openings {
        Openings {
            data: HashMap::default(),
        }
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
            let record: OpeningRecord = record.map_err(Arc::new)?;

            let mut pos = Chess::default();
            for token in record.pgn.split(' ') {
                if let Ok(san) = token.parse::<San>() {
                    pos.play_unchecked(&san.to_move(&pos)?);
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
        play: Vec<Uci>,
    ) -> Result<Option<Opening>, Error> {
        let mut opening = self.classify(root);

        for uci in play {
            let m = uci.to_move(root)?;
            root.play_unchecked(&m);

            opening = self.classify(root).or(opening);
        }

        Ok(opening.cloned())
    }

    fn classify(&self, pos: &VariantPosition) -> Option<&Opening> {
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
