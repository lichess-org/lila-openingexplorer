use crate::{
    model::{GameId, Mode, PersonalEntry, Speed, Stats, UserName, MAX_PERSONAL_GAMES},
    opening::Opening,
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, FromInto, StringWithSeparator};
use shakmaty::{
    fen::{Fen, ParseFenError},
    san::SanPlus,
    uci::Uci,
    variant::VariantPosition,
    Color,
};
use std::{cmp::Reverse, str::FromStr};

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
    total: Stats,
    moves: Vec<PersonalMoveRow>,
    #[serde_as(as = "Vec<DisplayFromStr>")]
    recent_games: Vec<GameId>,
    opening: Option<&'static Opening>,
}

#[serde_as]
#[derive(Serialize, Debug)]
struct PersonalMoveRow {
    #[serde_as(as = "DisplayFromStr")]
    uci: Uci,
    #[serde_as(as = "DisplayFromStr")]
    san: SanPlus,
    #[serde(flatten)]
    stats: Stats,
    #[serde_as(as = "Option<DisplayFromStr>")]
    game: Option<GameId>,
}

impl PersonalQueryFilter {
    pub fn respond(
        &self,
        pos: VariantPosition,
        entry: PersonalEntry,
        opening: Option<&'static Opening>,
    ) -> PersonalResponse {
        let mut total = Stats::default();
        let mut moves = Vec::with_capacity(entry.sub_entries.len());
        let mut recent_games: Vec<(u64, GameId)> = Vec::new();

        for (uci, sub_entry) in entry.sub_entries {
            let m = uci.to_move(&pos).expect("legal uci in personal entry");
            let san = SanPlus::from_move(pos.clone(), &m);

            let mut latest_game: Option<(u64, GameId)> = None;
            let mut stats = Stats::default();

            for speed in Speed::ALL {
                if self
                    .speeds
                    .as_ref()
                    .map_or(true, |speeds| speeds.contains(&speed))
                {
                    for mode in Mode::ALL {
                        if self
                            .modes
                            .as_ref()
                            .map_or(true, |modes| modes.contains(&mode))
                        {
                            let group = sub_entry.by_speed(speed).by_mode(mode);
                            stats += group.stats.to_owned();

                            for (idx, game) in group.games.iter().copied() {
                                if latest_game.map_or(true, |(latest_idx, _game)| latest_idx < idx)
                                {
                                    latest_game = Some((idx, game))
                                }
                            }

                            recent_games.extend(group.games.iter());
                        }
                    }
                }
            }

            total += stats.clone();

            moves.push(PersonalMoveRow {
                uci,
                san,
                stats,
                game: latest_game.map(|(_, id)| id),
            });
        }

        moves.sort_by_key(|row| row.stats.total());
        recent_games.sort_by_key(|(idx, _game)| Reverse(*idx));

        PersonalResponse {
            total,
            moves,
            recent_games: recent_games
                .into_iter()
                .map(|(_, game)| game)
                .take(MAX_PERSONAL_GAMES as usize)
                .collect(),
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
