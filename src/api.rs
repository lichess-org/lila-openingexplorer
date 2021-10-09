use crate::lila::LilaVariant;
use crate::model::{Mode, Speed};
use serde::Deserialize;
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, FromInto, StringWithSeparator};
use shakmaty::fen::{Fen, ParseFenError};
use shakmaty::uci::Uci;
use shakmaty::Color;
use std::fmt;
use std::str::FromStr;

#[serde_as]
#[derive(Deserialize)]
pub struct PersonalQuery {
    variant: LilaVariant,
    #[serde_as(as = "DisplayFromStr")]
    fen: LaxFen,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    play: Vec<Uci>,
    modes: Option<Vec<Mode>>,
    speeds: Option<Vec<Speed>>,
    player: String,
    #[serde_as(as = "FromInto<ColorProxy>")]
    color: Color,
    //since: _SinceYear,
}

struct _SinceYear(u8); // since 2000 or so

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum ColorProxy {
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
