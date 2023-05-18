use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr, TryFromInto};
use shakmaty::{san::SanPlus, uci::Uci, ByColor, Color};

use crate::{
    model::{
        GameId, GamePlayer, History, LichessGame, MastersGame, Mode, Month, Speed, Stats, Year,
    },
    opening::Opening,
    util::ByColorDef,
};

#[serde_as]
#[derive(Serialize, Clone, Debug)]
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
    pub queue_position: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<History>,
}

#[serde_as]
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerMove {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde_as(as = "DisplayFromStr")]
    pub san: SanPlus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_rating: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_opponent_rating: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<i32>,
    #[serde(flatten)]
    pub stats: Stats,
    pub game: Option<ExplorerGame>,
}

#[serde_as]
#[derive(Serialize, Clone, Debug)]
pub struct ExplorerGameWithUci {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: Uci,
    #[serde(flatten)]
    pub row: ExplorerGame,
}

#[serde_as]
#[derive(Serialize, Clone, Debug)]
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
    pub players: ByColor<GamePlayer>,
    #[serde_as(as = "TryFromInto<u16>")]
    pub year: Year,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub month: Option<Month>,
}

impl ExplorerGame {
    pub fn from_lichess(id: GameId, info: LichessGame) -> ExplorerGame {
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

    pub fn from_masters(id: GameId, info: MastersGame) -> ExplorerGame {
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
