use crate::model::{Mode, Speed};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, FromInto, StringWithSeparator};
use shakmaty::fen::{Fen, ParseFenError};
use shakmaty::uci::Uci;
use shakmaty::Color;
use std::error::Error as StdError;
use std::fmt;
use std::str::FromStr;

mod error;
mod variant;

pub use error::Error;
pub use variant::LilaVariant;

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
    #[serde_as(as = "DisplayFromStr")]
    pub player: UserName,
    #[serde_as(as = "FromInto<ColorProxy>")]
    color: Color,
    #[serde(default)]
    since: Option<SinceYear>,
}

#[derive(Serialize)]
pub struct PersonalResponse {}

#[derive(Deserialize, Default)]
struct SinceYear(u8); // since 2000 or so

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

#[derive(Debug)]
pub struct UserName(String);

impl fmt::Display for UserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug)]
pub struct InvalidUserName;

impl fmt::Display for InvalidUserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid username")
    }
}

impl StdError for InvalidUserName {}

impl FromStr for UserName {
    type Err = InvalidUserName;

    fn from_str(s: &str) -> Result<UserName, InvalidUserName> {
        if !s.is_empty()
            && s.len() <= 30
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            Ok(UserName(s.into()))
        } else {
            Err(InvalidUserName)
        }
    }
}
