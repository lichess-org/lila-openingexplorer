use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};
use shakmaty::{uci::Uci, ByColor, Color};

use crate::{model::GameId, util::ByColorDef};

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct MasterGameWithId {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde(flatten)]
    pub game: MasterGame,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub struct MasterGame {
    pub event: String,
    pub site: String,
    pub date: String,
    pub round: String,
    #[serde(flatten, with = "ByColorDef")]
    pub players: ByColor<MasterGamePlayer>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub winner: Option<Color>,
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, Uci>")]
    pub moves: Vec<Uci>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MasterGamePlayer {
    pub name: String,
    pub rating: u16,
}
