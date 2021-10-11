use crate::{
    model::{GameId, GameInfo, Mode, Speed, Stats, UserName},
    opening::Opening,
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, FromInto, StringWithSeparator};
use shakmaty::{
    fen::{Fen, ParseFenError},
    san::SanPlus,
    uci::Uci,
    Color,
};
use std::str::FromStr;

mod error;
mod variant;

pub use error::Error;
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
    #[serde_as(as = "FromInto<ColorProxy>")]
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorProxy {
    White,
    Black,
}

impl From<Color> for ColorProxy {
    fn from(color: Color) -> ColorProxy {
        match color {
            Color::White => ColorProxy::White,
            Color::Black => ColorProxy::Black,
        }
    }
}

impl From<ColorProxy> for Color {
    fn from(color: ColorProxy) -> Color {
        match color {
            ColorProxy::White => Color::White,
            ColorProxy::Black => Color::Black,
        }
    }
}

#[derive(Debug)]
pub struct LaxFen(Fen);

impl From<Fen> for LaxFen {
    fn from(fen: Fen) -> LaxFen {
        LaxFen(fen)
    }
}

impl From<LaxFen> for Fen {
    fn from(LaxFen(fen): LaxFen) -> Fen {
        fen
    }
}

impl FromStr for LaxFen {
    type Err = ParseFenError;

    fn from_str(s: &str) -> Result<LaxFen, ParseFenError> {
        s.replace("_", " ").parse().map(LaxFen)
    }
}
