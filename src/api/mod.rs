use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, StringWithSeparator};
use shakmaty::{san::SanPlus, uci::Uci, Color};

use crate::{
    model::{GameId, GameInfo, Mode, Month, Speed, Stats, UserName},
    opening::Opening,
};

mod error;
mod fen;
mod nd_json;
mod variant;

pub use error::Error;
pub use fen::LaxFen;
pub use nd_json::NdJson;
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
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct PersonalQueryFilter {
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Mode>>")]
    #[serde(default)]
    pub modes: Option<Vec<Mode>>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
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
#[serde(rename_all = "camelCase")]
pub struct PersonalMoveRow {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde_as(as = "DisplayFromStr")]
    pub san: SanPlus,
    pub average_opponent_rating: Option<u64>,
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
