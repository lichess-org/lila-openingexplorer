use std::collections::HashMap;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use shakmaty::{
    fen::Fen,
    uci::Uci,
    variant::{Variant, VariantPosition},
    zobrist::{Zobrist, ZobristHash},
    CastlingMode, Chess, FromSetup, Position,
};

use crate::api::Error;

#[derive(Serialize, Debug)]
pub struct Opening {
    eco: String,
    name: String,
}

#[serde_as]
#[derive(Deserialize)]
struct OpeningRecord {
    eco: String,
    name: String,
    #[serde_as(as = "DisplayFromStr")]
    epd: Fen,
}

impl From<OpeningRecord> for Opening {
    fn from(record: OpeningRecord) -> Opening {
        Opening {
            eco: record.eco,
            name: record.name,
        }
    }
}

pub struct Openings {
    data: FxHashMap<u128, Opening>,
}

impl Openings {
    pub fn build_table() -> Openings {
        let mut data = HashMap::with_hasher(Default::default());

        for tsv in TSV_DATA {
            let mut tsv = csv::ReaderBuilder::new().delimiter(b'\t').from_reader(tsv);
            for record in tsv.deserialize() {
                let record: OpeningRecord = record.expect("valid opening tsv");
                assert!(
                    data.insert(
                        Chess::from_setup(&record.epd, CastlingMode::Chess960)
                            .expect("legal opening position")
                            .zobrist_hash(),
                        Opening::from(record),
                    )
                    .is_none(),
                    "zobrist hash unique on openings"
                );
            }
        }

        Openings { data }
    }

    pub fn classify_and_play(
        &self,
        root: &mut Zobrist<VariantPosition, u128>,
        play: Vec<Uci>,
    ) -> Result<Option<&Opening>, Error> {
        let mut opening = None;

        for uci in play {
            let m = uci.to_move(root)?;
            root.play_unchecked(&m);

            if opening_sensible(root.as_inner().variant()) {
                opening = self.data.get(&root.zobrist_hash()).or(opening);
            }
        }

        Ok(opening)
    }
}

const TSV_DATA: [&[u8]; 5] = [
    include_bytes!("../chess-openings/dist/a.tsv"),
    include_bytes!("../chess-openings/dist/b.tsv"),
    include_bytes!("../chess-openings/dist/c.tsv"),
    include_bytes!("../chess-openings/dist/d.tsv"),
    include_bytes!("../chess-openings/dist/e.tsv"),
];

fn opening_sensible(variant: Variant) -> bool {
    matches!(
        variant,
        Variant::Chess | Variant::Crazyhouse | Variant::ThreeCheck | Variant::KingOfTheHill
    )
}
