use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, StringWithSeparator};
use shakmaty::{san::SanPlus, uci::Uci, Color};

use crate::{
    model::{GameId, GameInfo, Mode, Month, Speed, Stats, UserName},
    opening::Opening,
};

mod error;
mod fen;
mod variant;

pub use error::Error;
pub use fen::LaxFen;
pub use variant::LilaVariant;

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
    #[serde(flatten)]
    pub filter: PersonalQueryFilter,
    #[serde(default)]
    pub update: bool,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct PersonalQueryFilter {
    #[serde(default)]
    pub modes: Option<Vec<Mode>>,
    #[serde(default)]
    pub speeds: Option<Vec<Speed>>,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default)]
    pub since: Month,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "Month::max_value")]
    pub until: Month,
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
