use shakmaty::fen::Fen;
use shakmaty::uci::Uci;
use shakmaty::Color;

use serde::Deserialize;
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, FromInto, StringWithSeparator};

use crate::lila::LilaVariant;
use crate::model::{Mode, Speed};

#[serde_as]
#[derive(Deserialize)]
pub struct PersonalQuery {
    variant: LilaVariant,
    #[serde_as(as = "DisplayFromStr")]
    fen: Fen,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    play: Vec<Uci>,
    modes: Option<Vec<Mode>>,
    speeds: Option<Vec<Speed>>,
    player: String,
    #[serde_as(as = "FromInto<LaxColor>")]
    color: Color,
    //since: _SinceYear,
}

struct _SinceYear(u8); // since 2000 or so

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum LaxColor {
    #[serde(alias = "w")]
    White,
    #[serde(alias = "b")]
    Black,
}

impl From<Color> for LaxColor {
    fn from(color: Color) -> LaxColor {
        match color {
            Color::White => LaxColor::White,
            Color::Black => LaxColor::Black,
        }
    }
}

impl From<LaxColor> for Color {
    fn from(color: LaxColor) -> Color {
        match color {
            LaxColor::White => Color::White,
            LaxColor::Black => Color::Black,
        }
    }
}
