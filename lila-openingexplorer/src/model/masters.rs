use std::{
    cmp::{min, Reverse},
    io,
    io::{Cursor, Write},
};

use axum::{
    body::Body,
    response::{IntoResponse, Response},
};
use bytes::{Buf, BufMut};
use nohash_hasher::IntMap;
use serde::{Deserialize, Serialize};
use serde_with::{formats::SpaceSeparator, serde_as, DisplayFromStr, StringWithSeparator};
use shakmaty::{san::SanPlus, uci::UciMove, ByColor, Chess, Color, Outcome};
use thin_vec::{thin_vec, ThinVec};

use crate::{
    api::Limits,
    model::{GameId, GamePlayer, LaxDate, PreparedMove, PreparedResponse, RawUciMove, Stats},
    util::{sort_by_key_and_truncate, ByColorDef},
};

const MAX_MASTERS_GAMES: usize = 15;

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct MastersGameWithId {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde(flatten)]
    pub game: MastersGame,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub struct MastersGame {
    pub event: String,
    pub site: String,
    #[serde_as(as = "DisplayFromStr")]
    pub date: LaxDate,
    pub round: String,
    #[serde(flatten, with = "ByColorDef")]
    pub players: ByColor<GamePlayer>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub winner: Option<Color>,
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, UciMove>")]
    pub moves: Vec<UciMove>,
}

impl MastersGame {
    fn outcome(&self) -> Outcome {
        Outcome::from_winner(self.winner)
    }

    fn write_pgn<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writeln!(writer, "[Event \"{}\"]", self.event)?;
        writeln!(writer, "[Site \"{}\"]", self.site)?;
        writeln!(writer, "[Date \"{}\"]", self.date)?;
        writeln!(writer, "[Round \"{}\"]", self.round)?;
        writeln!(writer, "[White \"{}\"]", self.players.white.name)?;
        writeln!(writer, "[Black \"{}\"]", self.players.black.name)?;
        writeln!(writer, "[Result \"{}\"]", self.outcome())?;
        writeln!(writer, "[WhiteElo \"{}\"]", self.players.white.rating)?;
        writeln!(writer, "[BlackElo \"{}\"]", self.players.black.rating)?;
        writeln!(writer)?;

        let mut pos = Chess::default();

        for (i, uci) in self.moves.iter().enumerate() {
            let m = uci
                .to_move(&pos)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            if i % 2 == 0 {
                if i > 0 {
                    write!(writer, " ")?;
                }
                write!(writer, "{}.", i / 2 + 1)?;
            }
            let san = SanPlus::from_move_and_play_unchecked(&mut pos, &m);
            write!(writer, " {san}")?;
        }

        if !self.moves.is_empty() {
            write!(writer, " ")?;
        }
        writeln!(writer, "{}", self.outcome())
    }
}

impl IntoResponse for MastersGame {
    fn into_response(self) -> Response {
        let mut buf = Cursor::new(Vec::new());
        self.write_pgn(&mut buf).expect("write pgn");

        Response::builder()
            .header(axum::http::header::CONTENT_TYPE, "application/x-chess-pgn")
            .body(Body::from(buf.into_inner()))
            .unwrap()
    }
}

#[derive(Debug, Default)]
pub struct MastersGroup {
    stats: Stats,
    games: ThinVec<(u16, GameId)>,
}

#[derive(Default, Debug)]
pub struct MastersEntry {
    groups: IntMap<RawUciMove, MastersGroup>,
}

impl MastersEntry {
    pub const SIZE_HINT: usize = 14;

    pub fn new_single(
        uci: UciMove,
        id: GameId,
        outcome: Outcome,
        mover_rating: u16,
        opponent_rating: u16,
    ) -> MastersEntry {
        MastersEntry {
            groups: [(
                RawUciMove::from(uci),
                MastersGroup {
                    stats: Stats::new_single(outcome, mover_rating),
                    games: thin_vec![(mover_rating.saturating_add(opponent_rating), id)],
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    pub fn extend_from_reader<B: Buf>(&mut self, buf: &mut B) {
        while buf.has_remaining() {
            let uci = RawUciMove::read(buf);
            let group = self.groups.entry(uci).or_default();
            group.stats += &Stats::read(buf);
            let num_games = usize::from(buf.get_u8());
            group
                .games
                .extend((0..num_games).map(|_| (buf.get_u16_le(), GameId::read(buf))));
        }
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        let mut top_games: Vec<_> = self
            .groups
            .values()
            .flat_map(|group| group.games.iter().copied())
            .collect();

        let lowest_top_game = if top_games.len() > MAX_MASTERS_GAMES {
            let (_, lowest_top_game, _) =
                top_games.select_nth_unstable_by_key(MAX_MASTERS_GAMES - 1, |g| Reverse(*g));
            lowest_top_game
        } else if let Some(lowest_top_game) = top_games.iter().min() {
            lowest_top_game
        } else {
            return;
        };

        for (uci, group) in &self.groups {
            uci.write(buf);
            group.stats.write(buf);

            let num_games = if group.games.len() == 1 {
                1
            } else {
                group.games.iter().filter(|g| *g >= lowest_top_game).count()
            };
            buf.put_u8(num_games as u8);

            for (sort_key, id) in group
                .games
                .iter()
                .filter(|g| group.games.len() == 1 || *g >= lowest_top_game)
            {
                buf.put_u16_le(*sort_key);
                id.write(buf);
            }
        }
    }

    pub fn prepare(self, limits: &Limits) -> PreparedResponse {
        let mut total = Stats::default();
        let mut moves = Vec::with_capacity(self.groups.len());
        let mut top_games = Vec::new();

        for (uci, group) in self.groups {
            total += &group.stats;

            let uci = UciMove::from(uci);

            let single_game = if group.stats.is_single() {
                group.games.iter().map(|(_, id)| *id).next()
            } else {
                None
            };
            moves.push(PreparedMove {
                uci: uci.clone(),
                average_rating: group.stats.average_rating(),
                average_opponent_rating: None,
                performance: None,
                game: single_game,
                stats: group.stats,
            });

            top_games.extend(
                group
                    .games
                    .iter()
                    .copied()
                    .map(|(sort_key, game)| (sort_key, uci.clone(), game)),
            );
        }

        sort_by_key_and_truncate(
            &mut top_games,
            min(limits.top_games, MAX_MASTERS_GAMES),
            |(sort_key, _, _)| Reverse(*sort_key),
        );

        sort_by_key_and_truncate(&mut moves, limits.moves, |m| Reverse(m.stats.total()));

        PreparedResponse {
            total,
            moves,
            top_games: top_games
                .into_iter()
                .map(|(_, uci, game)| (uci, game))
                .collect(),
            recent_games: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::Square;

    use super::*;

    #[test]
    fn test_masters_entry() {
        let uci = UciMove::Normal {
            from: Square::E2,
            to: Square::E4,
            promotion: None,
        };
        let game = "aaaaaaaa".parse().unwrap();
        let a = MastersEntry::new_single(uci.clone(), game, Outcome::Draw, 1600, 1700);

        let mut buf = Vec::with_capacity(MastersEntry::SIZE_HINT);
        a.write(&mut buf);
        assert_eq!(
            buf.len(),
            MastersEntry::SIZE_HINT,
            "optimized for single entries"
        );

        let mut reader = &buf[..];
        let mut deserialized = MastersEntry::default();
        deserialized.extend_from_reader(&mut reader);

        let group = deserialized.groups.get(&RawUciMove::from(uci)).unwrap();
        assert_eq!(group.stats.draws(), 1);
        assert_eq!(group.games[0], (1600 + 1700, game));
    }
}
