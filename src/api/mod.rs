use crate::{
    model::{Mode, Speed, UserName, Stats, PersonalEntry},
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

#[derive(Serialize, Debug)]
pub struct PersonalResponse {
    #[serde(flatten)]
    pub total: Stats,
    pub opening: Option<&'static Opening>,
}

impl PersonalQueryFilter {
    pub fn respond(&self, entry: PersonalEntry, opening: Option<&'static Opening>) -> PersonalResponse {
        let mut total = Stats::default();

        for (uci, sub_entry) in entry.sub_entries {
            for speed in Speed::ALL {
                if self.speeds.as_ref().map_or(true, |speeds| speeds.contains(&speed)) {
                    for mode in Mode::ALL {
                        if self.modes.as_ref().map_or(true, |modes| modes.contains(&mode)) {
                            let group = sub_entry.by_speed(speed).by_mode(mode);
                            total += group.stats.to_owned();
                        }
                    }
                }
            }
        }

        PersonalResponse {
            total,
            opening,
        }
    }
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
