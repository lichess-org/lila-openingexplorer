use serde::Serialize;
use serde_with::{DisplayFromStr, TryFromInto, serde_as};
use shakmaty::{Color, san::SanPlus, uci::UciMove};

use crate::{
    packed::{BorrowedLichessGame, GameId, GamePlayer, Month, Rating, RatingMode, Speed, SubStats},
    server::openings::Opening,
};

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct ExplorerGame {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub winner: Option<Color>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<Speed>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<RatingMode>,
    pub black: GamePlayer,
    pub white: GamePlayer,
    pub year: u32,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub month: Option<Month>,
}

impl ExplorerGame {
    pub fn new(id: GameId, game: &BorrowedLichessGame<'_>, masters: bool) -> ExplorerGame {
        ExplorerGame {
            id,
            winner: game.outcome.winner(),
            speed: if masters { None } else { game.speed },
            mode: (!masters).then_some(game.rating_mode),
            black: game.players.black.into_owned(),
            white: game.players.white.into_owned(),
            year: game.date.year(),
            month: if masters {
                game.date
                    .month()
                    .map(|m| Month::from_year_and_month(game.date.year() as u16, u16::from(m)))
                    .and_then(Result::ok)
            } else {
                Some(game.date.earliest_month())
            },
        }
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct ExplorerGameWithUci {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: UciMove,
    #[serde(flatten)]
    pub game: ExplorerGame,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct Stats {
    pub white: u64,
    pub draws: u64,
    pub black: u64,
}

impl From<&SubStats> for Stats {
    fn from(stats: &SubStats) -> Stats {
        Stats {
            white: stats.white,
            draws: stats.draws,
            black: stats.black,
        }
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerMove {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: UciMove,
    #[serde_as(as = "DisplayFromStr")]
    pub san: SanPlus,
    #[serde_as(as = "Option<TryFromInto<i32>>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_rating: Option<Rating>,
    #[serde(flatten)]
    pub stats: Stats,
    pub game: Option<ExplorerGame>,
    pub opening: Option<Opening>,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    #[serde_as(as = "DisplayFromStr")]
    pub month: Month,
    #[serde(flatten)]
    pub stats: Stats,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerResponse {
    #[serde(flatten)]
    pub total: Stats,
    pub moves: Vec<ExplorerMove>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_games: Option<Vec<ExplorerGameWithUci>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_games: Option<Vec<ExplorerGameWithUci>>,
    pub opening: Option<Opening>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<HistoryEntry>>,
}
