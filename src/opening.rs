use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use shakmaty::{fen::Fen, zobrist::ZobristHash, CastlingMode, Chess, FromSetup};
use std::collections::HashMap;

#[derive(Serialize)]
struct Opening {
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
    pub fn new() -> Openings {
        let mut data = HashMap::with_hasher(Default::default());

        for tsv in TSV_DATA {
            let mut tsv = csv::ReaderBuilder::new().delimiter(b'\t').from_reader(tsv);
            for record in tsv.deserialize() {
                let record: OpeningRecord = record.expect("valid opening tsv");
                data.insert(
                    Chess::from_setup(&record.epd, CastlingMode::Chess960)
                        .expect("legal opening position")
                        .zobrist_hash(),
                    Opening::from(record),
                );
            }
        }

        Openings { data }
    }
}

const TSV_DATA: [&[u8]; 5] = [
    include_bytes!("../chess-openings/dist/a.tsv"),
    include_bytes!("../chess-openings/dist/b.tsv"),
    include_bytes!("../chess-openings/dist/c.tsv"),
    include_bytes!("../chess-openings/dist/d.tsv"),
    include_bytes!("../chess-openings/dist/e.tsv"),
];
