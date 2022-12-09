use std::collections::HashMap;

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

#[derive(Serialize, Debug)]
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

pub struct Openings {
    data: IntMap<Zobrist64, Opening>,
}

impl Openings {
    pub fn build_table() -> Openings {
        let mut data = HashMap::default();

        for tsv in TSV_DATA {
            let mut tsv = csv::ReaderBuilder::new().delimiter(b'\t').from_reader(tsv);
            for record in tsv.deserialize() {
                let record: OpeningRecord = record.expect("valid opening tsv");

                let mut pos = Chess::default();
                for token in record.pgn.split(' ') {
                    if let Ok(san) = token.parse::<San>() {
                        pos.play_unchecked(&san.to_move(&pos).expect("legal move"));
                    }
                }

                assert!(
                    data.insert(
                        pos.zobrist_hash(EnPassantMode::Legal),
                        Opening {
                            eco: record.eco,
                            name: record.name,
                        }
                    )
                    .is_none(),
                    "zobrist hash unique on openings"
                );
            }
        }

        data.shrink_to_fit();
        Openings { data }
    }

    pub fn classify_and_play(
        &self,
        root: &mut VariantPosition,
        play: Vec<Uci>,
    ) -> Result<Option<&Opening>, Error> {
        let mut opening = self.classify(root);

        for uci in play {
            let m = uci.to_move(root)?;
            root.play_unchecked(&m);

            opening = self.classify(root).or(opening);
        }

        Ok(opening)
    }

    fn classify(&self, pos: &VariantPosition) -> Option<&Opening> {
        if opening_sensible(pos.variant()) {
            self.data.get(&pos.zobrist_hash(EnPassantMode::Legal))
        } else {
            None
        }
    }
}

const TSV_DATA: [&[u8]; 5] = [
    include_bytes!("../chess-openings/a.tsv"),
    include_bytes!("../chess-openings/b.tsv"),
    include_bytes!("../chess-openings/c.tsv"),
    include_bytes!("../chess-openings/d.tsv"),
    include_bytes!("../chess-openings/e.tsv"),
];

fn opening_sensible(variant: Variant) -> bool {
    matches!(
        variant,
        Variant::Chess | Variant::Crazyhouse | Variant::ThreeCheck | Variant::KingOfTheHill
    )
}
