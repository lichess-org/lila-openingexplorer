use crate::{
    model::{GameId, GameInfo, Mode, Speed, Stats, UserName},
    opening::Opening,
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, StringWithSeparator};
use shakmaty::{
    san::SanPlus,
    uci::Uci,
    Color,
};

mod error;
mod variant;
mod fen;

pub use error::Error;
pub use variant::LilaVariant;
pub use fen::LaxFen;

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct PersonalQuery {
    #[serde(default)]
    pub variant: LilaVariant,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub fen: Option<LaxFen>,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    pub play: Vec<Uci>,
    #[serde_as(as = "DisplayFromStr")]
    pub player: UserName,
    #[serde_as(as = "DisplayFromStr")]
    pub color: Color,
    #[serde(default)]
    pub since: u32, // year
    #[serde(flatten)]
    pub filter: PersonalQueryFilter,
    #[serde(default)]
    pub update: bool,
}

#[derive(Deserialize, Debug)]
pub struct PersonalQueryFilter {
    #[serde(default)]
    pub modes: Option<Vec<Mode>>,
    #[serde(default)]
    pub speeds: Option<Vec<Speed>>,
}

#[serde_as]
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PersonalResponse {
    #[serde(flatten)]
    pub total: Stats,
    pub moves: Vec<PersonalMoveRow>,
    pub recent_games: Vec<GameRowWithUci>,
    pub opening: Option<&'static Opening>,
}

#[serde_as]
#[derive(Serialize, Debug)]
pub struct PersonalMoveRow {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde_as(as = "DisplayFromStr")]
    pub san: SanPlus,
    #[serde(flatten)]
    pub stats: Stats,
    pub game: Option<GameRow>,
}

#[serde_as]
#[derive(Serialize, Debug)]
pub struct GameRowWithUci {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde(flatten)]
    pub row: GameRow,
}

#[serde_as]
#[derive(Serialize, Debug)]
pub struct GameRow {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde(flatten)]
    pub info: GameInfo,
}
