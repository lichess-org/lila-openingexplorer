use crate::{
    model::{Mode, Speed, UserName},
    opening::Opening,
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, FromInto, StringWithSeparator};
use shakmaty::fen::{Fen, ParseFenError};
use shakmaty::uci::Uci;
use shakmaty::Color;
use std::str::FromStr;

mod error;
mod variant;

pub use error::Error;
pub use variant::LilaVariant;

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct PersonalQuery {
    pub variant: LilaVariant,
    #[serde_as(as = "DisplayFromStr")]
    pub fen: LaxFen,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    pub play: Vec<Uci>,
    pub modes: Option<Vec<Mode>>,
    pub speeds: Option<Vec<Speed>>,
    #[serde_as(as = "DisplayFromStr")]
    pub player: UserName,
    #[serde_as(as = "FromInto<ColorProxy>")]
    pub color: Color,
    #[serde(default)]
    pub since: u32, // year
    #[serde(default)]
    pub update: bool,
}

#[derive(Serialize)]
pub struct PersonalResponse {
    pub opening: Option<&'static Opening>,
}

#[derive(Deserialize)]
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
