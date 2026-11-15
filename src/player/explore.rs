use std::cmp::Reverse;

use serde::Serialize;
use serde_with::{DisplayFromStr, serde_as};
use shakmaty::{Color, packed::PackedUciMove, san::SanPlus, uci::UciMove};

use crate::{
    packed::{EntryView, GameId, Month, PlayerGroupHeader, RatingMode, Speed, SubStats},
    player::{
        game::PlayerGame,
        keys::PlayerGameIndex,
        query::{PlayerFilter, PlayerLimits},
    },
    server::{openings::Opening, response::Stats},
};

#[derive(Debug, Clone)]
pub struct PreparedMove {
    pub m: PackedUciMove,
    pub stats: SubStats,
    /// The only game, if the move was played exactly once.
    pub game: Option<PlayerGameIndex>,
}

#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub total: SubStats,
    /// Most played first.
    pub moves: Vec<PreparedMove>,
    /// Newest first.
    pub recent_games: Vec<(PackedUciMove, PlayerGameIndex)>,
}

pub fn prepare(entry: &[u8], filter: &PlayerFilter, limits: &PlayerLimits) -> Prepared {
    let mut prepared = Prepared::default();
    let mut recent: Vec<(PlayerGameIndex, PackedUciMove)> = Vec::new();
    let mut current: Option<(PreparedMove, Option<PlayerGameIndex>)> = None;
    let finish = |current: Option<(PreparedMove, Option<PlayerGameIndex>)>,
                  prepared: &mut Prepared| {
        if let Some((mut m, latest)) = current
            && m.stats.games() > 0
        {
            m.game = latest.filter(|_| m.stats.games() == 1);
            prepared.total.merge(&m.stats);
            prepared.moves.push(m);
        }
    };
    for group in EntryView::<PlayerGroupHeader>::unpack_entire(entry).groups() {
        if current
            .as_ref()
            .is_none_or(|(m, _)| m.m != group.uci_move())
        {
            finish(current.take(), &mut prepared);
            current = Some((
                PreparedMove {
                    m: group.uci_move(),
                    stats: SubStats::default(),
                    game: None,
                },
                None,
            ));
        }
        if !filter.contains_speed(group.header.speed) || !filter.contains_mode(group.header.mode) {
            continue;
        }
        let (m, latest) = current.as_mut().expect("current move");
        let mut cells = group.cells();
        while let Some(cell) = cells.next_cell() {
            let month = cell.key.month();
            if filter.since.is_some_and(|since| month < since) {
                // Months are newest first.
                break;
            }
            if !filter.contains_month(month) {
                continue;
            }
            m.stats.merge(&cell.stats);
            while let Some(game) = cells.next_game() {
                let index = PlayerGameIndex::try_from(game).expect("player game index");
                *latest = (*latest).max(Some(index));
                recent.push((index, m.m));
            }
        }
    }
    finish(current.take(), &mut prepared);

    prepared.moves.sort_by_key(|m| Reverse(m.stats.games()));
    prepared.moves.truncate(limits.moves);
    recent.sort_unstable_by_key(|&(index, _)| Reverse(index));
    recent.truncate(limits.recent_games);
    prepared.recent_games = recent.into_iter().map(|(index, m)| (m, index)).collect();
    prepared
}

#[derive(Serialize, Debug, Clone)]
pub struct PlayerGamePlayer {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rating: i32,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct PlayerGameResponse {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub winner: Option<Color>,
    #[serde_as(as = "DisplayFromStr")]
    pub speed: Speed,
    #[serde_as(as = "DisplayFromStr")]
    pub mode: RatingMode,
    pub white: PlayerGamePlayer,
    pub black: PlayerGamePlayer,
    pub year: u32,
    #[serde_as(as = "DisplayFromStr")]
    pub month: Month,
}

impl PlayerGameResponse {
    pub fn new(game: &PlayerGame, name: &str) -> PlayerGameResponse {
        let me = PlayerGamePlayer {
            name: name.to_owned(),
            title: game.player_title.clone(),
            rating: i32::from(game.player_rating),
        };
        let opponent = PlayerGamePlayer {
            name: game.opponent_name.clone(),
            title: game.opponent_title.clone(),
            rating: i32::from(game.opponent_rating),
        };
        let (white, black) = match game.color {
            Color::White => (me, opponent),
            Color::Black => (opponent, me),
        };
        PlayerGameResponse {
            id: game.id,
            winner: game.outcome.winner(),
            speed: game.speed,
            mode: game.mode,
            white,
            black,
            year: u32::from(game.month.months_since_jan_1900() / 12) + 1900,
            month: game.month,
        }
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayerMove {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: UciMove,
    #[serde_as(as = "DisplayFromStr")]
    pub san: SanPlus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_opponent_rating: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<i32>,
    #[serde(flatten)]
    pub stats: Stats,
    pub game: Option<PlayerGameResponse>,
    pub opening: Option<Opening>,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct PlayerGameWithUci {
    #[serde_as(as = "DisplayFromStr")]
    pub uci: UciMove,
    #[serde(flatten)]
    pub game: PlayerGameResponse,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub opening: Option<Opening>,
    pub queue_position: u64,
    #[serde(flatten)]
    pub total: Stats,
    pub moves: Vec<PlayerMove>,
    pub recent_games: Vec<PlayerGameWithUci>,
}

impl PlayerResponse {
    pub fn total_games(&self) -> u64 {
        self.total.white + self.total.draws + self.total.black
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::KnownOutcome;

    use super::*;
    use crate::packed::{CellKey, GameIndex, PlayerEntry, Rating, RatingModes};

    #[test]
    fn test_prepare() {
        let mut entry = PlayerEntry::new();
        let e4 = PackedUciMove::pack("e2e4".parse().unwrap());
        let d4 = PackedUciMove::pack("d2d4".parse().unwrap());
        let blitz = PlayerGroupHeader {
            speed: Speed::Blitz,
            mode: RatingMode::Rated,
        };
        let casual = PlayerGroupHeader {
            speed: Speed::Blitz,
            mode: RatingMode::Casual,
        };
        let month = |m| Month::from_months_since_jan_1900(1400 + m);
        let rating = Rating::try_from(1600).unwrap();
        let win = KnownOutcome::Decisive {
            winner: Color::White,
        };
        for index in 0..10u64 {
            entry.add_game(
                CellKey::new(e4, blitz, month((index % 3) as u16)),
                rating,
                win,
                GameIndex::new(index).unwrap(),
            );
        }
        entry.add_game(
            CellKey::new(d4, casual, month(5)),
            rating,
            KnownOutcome::Draw,
            GameIndex::new(10).unwrap(),
        );
        let mut buf = Vec::new();
        entry.pack_entire(&mut buf);
        let limits = PlayerLimits {
            moves: usize::MAX,
            recent_games: 8,
        };
        let prepared = prepare(&buf, &PlayerFilter::default(), &limits);
        assert_eq!(prepared.total.games(), 11);
        assert_eq!(prepared.moves[0].m, e4);
        assert_eq!(prepared.moves[0].game, None);
        assert_eq!(prepared.moves[1].game, Some(PlayerGameIndex(10)));
        assert_eq!(prepared.recent_games.len(), 8);
        assert_eq!(prepared.recent_games[0], (d4, PlayerGameIndex(10)));
        assert_eq!(prepared.recent_games[1], (e4, PlayerGameIndex(9)));
        assert_eq!(
            prepared.moves[0].stats.average_rating().map(i32::from),
            Some(1600)
        );
        assert_eq!(
            prepared.moves[0].stats.performance(Color::White),
            Some(2400)
        );
        assert_eq!(prepared.moves[0].stats.performance(Color::Black), Some(800));

        let rated = PlayerFilter {
            modes: Some(RatingModes::RATED),
            since: Some(month(1)),
            ..PlayerFilter::default()
        };
        let prepared = prepare(&buf, &rated, &limits);
        // Months 1 and 2 of e4: games 1, 2, 4, 5, 7, 8.
        assert_eq!(prepared.total.games(), 6);
        assert_eq!(prepared.moves.len(), 1);
        assert_eq!(prepared.recent_games[0], (e4, PlayerGameIndex(8)));
    }
}
