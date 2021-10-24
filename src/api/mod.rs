use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, StringWithSeparator, TryFromInto};
use shakmaty::{san::SanPlus, uci::Uci, ByColor, Color};

use crate::{
    model::{
        GameId, GameInfo, GameInfoPlayer, MasterGame, Mode, Month, RatingGroup, Speed, Stats,
        UserName, Year,
    },
    opening::Opening,
    util::ByColorDef,
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
pub struct MasterQuery {
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub fen: Option<LaxFen>,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    pub play: Vec<Uci>,
    #[serde_as(as = "TryFromInto<u16>")]
    #[serde(default)]
    pub since: Year,
    #[serde_as(as = "TryFromInto<u16>")]
    #[serde(default = "Year::max_value")]
    pub until: Year,
    #[serde(flatten)]
    pub limits: Limits,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct LichessQuery {
    #[serde(default)]
    pub variant: LilaVariant,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub fen: Option<LaxFen>,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    pub play: Vec<Uci>,
    #[serde(flatten)]
    pub limits: Limits,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
    #[serde(default)]
    pub speeds: Option<Vec<Speed>>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, RatingGroup>>")]
    #[serde(default)]
    pub ratings: Option<Vec<RatingGroup>>,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default)]
    pub since: Month,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "Month::max_value")]
    pub until: Month,
}

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
    #[serde(flatten)]
    pub limits: Limits,
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
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub top_games: usize,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub recent_games: usize,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub moves: Option<usize>,
}

#[serde_as]
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerResponse {
    #[serde(flatten)]
    pub total: Stats,
    pub moves: Vec<ExplorerMove>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_games: Option<Vec<ExplorerGameWithUci>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_games: Option<Vec<ExplorerGameWithUci>>,
    pub opening: Option<&'static Opening>,
}

#[serde_as]
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerMove {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde_as(as = "DisplayFromStr")]
    pub san: SanPlus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_rating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_opponent_rating: Option<u64>,
    #[serde(flatten)]
    pub stats: Stats,
    pub game: Option<ExplorerGame>,
}

#[serde_as]
#[derive(Serialize, Debug)]
pub struct ExplorerGameWithUci {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde(flatten)]
    pub row: ExplorerGame,
}

#[serde_as]
#[derive(Serialize, Debug)]
pub struct ExplorerGame {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub winner: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<Speed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(flatten, with = "ByColorDef")]
    pub players: ByColor<GameInfoPlayer>,
    #[serde_as(as = "TryFromInto<u16>")]
    pub year: Year,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub month: Option<Month>,
}

impl ExplorerGame {
    pub fn from_lichess(id: GameId, info: GameInfo) -> ExplorerGame {
        ExplorerGame {
            id,
            winner: info.outcome.winner(),
            speed: Some(info.speed),
            mode: Some(info.mode),
            players: info.players,
            year: info.month.year(),
            month: Some(info.month),
        }
    }

    pub fn from_master(id: GameId, info: MasterGame) -> ExplorerGame {
        ExplorerGame {
            id,
            winner: info.winner,
            speed: None,
            mode: None,
            players: info.players,
            year: info.date.year(),
            month: info.date.month(),
        }
    }
}
