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
