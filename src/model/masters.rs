use std::{
    cmp::{min, Reverse},
    io,
    io::{Cursor, Write},
    ops::AddAssign,
};

use axum::{
    body,
    response::{IntoResponse, Response},
};
use bytes::{Buf, BufMut};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};
use shakmaty::{san::SanPlus, uci::Uci, ByColor, Chess, Color, Outcome};

use crate::{
    api::Limits,
    model::{
        read_uci, write_uci, GameId, GamePlayer, LaxDate, PreparedMove, PreparedResponse, Stats,
    },
    util::{sort_by_key_and_truncate, ByColorDef},
};

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
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, Uci>")]
    pub moves: Vec<Uci>,
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
            write!(writer, " {}", san)?;
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
            .body(body::boxed(body::Full::from(buf.into_inner())))
            .unwrap()
    }
}

#[derive(Debug, Default)]
pub struct MastersGroup {
    pub stats: Stats,
    pub games: Vec<(u16, GameId)>,
}

impl AddAssign for MastersGroup {
    fn add_assign(&mut self, rhs: MastersGroup) {
        self.stats += rhs.stats;
        self.games.extend(rhs.games);
    }
}

#[derive(Default, Debug)]
pub struct MastersEntry {
    pub groups: FxHashMap<Uci, MastersGroup>,
}

impl MastersEntry {
    pub const SIZE_HINT: usize = 14;

    pub fn new_single(
        uci: Uci,
        id: GameId,
        outcome: Outcome,
        mover_rating: u16,
        opponent_rating: u16,
    ) -> MastersEntry {
        let mut groups = FxHashMap::with_capacity_and_hasher(1, Default::default());
        groups.insert(
            uci,
            MastersGroup {
                stats: Stats::new_single(outcome, mover_rating),
                games: vec![(mover_rating.saturating_add(opponent_rating), id)],
            },
        );
        MastersEntry { groups }
    }

    pub fn extend_from_reader<B: Buf>(&mut self, buf: &mut B) {
        while buf.has_remaining() {
            let uci = read_uci(buf);
            let group = self.groups.entry(uci).or_default();
            group.stats += Stats::read(buf);
            let num_games = usize::from(buf.get_u8());
            group.games.reserve(num_games);
            for _ in 0..num_games {
                group.games.push((buf.get_u16_le(), GameId::read(buf)));
            }
        }
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        let mut top_games = Vec::new();
        for group in self.groups.values() {
            top_games.extend(&group.games);
        }
        sort_by_key_and_truncate(&mut top_games, 15, |(sort_key, _)| Reverse(*sort_key));

        for (uci, group) in &self.groups {
            write_uci(buf, uci);

            group.stats.write(buf);

            let num_games = if group.games.len() == 1 {
                1
            } else {
                group.games.iter().filter(|g| top_games.contains(g)).count()
            };
            buf.put_u8(num_games as u8);
            for (sort_key, id) in group
                .games
                .iter()
                .filter(|g| group.games.len() == 1 || top_games.contains(g))
            {
                buf.put_u16_le(*sort_key);
                id.write(buf);
            }
        }
    }

    fn total(&self) -> Stats {
        let mut sum = Stats::default();
        for group in self.groups.values() {
            sum += group.stats.clone();
        }
        sum
    }

    pub fn prepare(self, limits: &Limits) -> PreparedResponse {
        let total = self.total();

        let mut top_games = Vec::new();
        for (uci, group) in &self.groups {
            for (sort_key, game) in &group.games {
                top_games.push((*sort_key, uci.to_owned(), *game));
            }
        }
        sort_by_key_and_truncate(
            &mut top_games,
            min(limits.top_games, 15),
            |(sort_key, _, _)| Reverse(*sort_key),
        );

        let mut moves: Vec<PreparedMove> = self
            .groups
            .into_iter()
            .map(|(uci, group)| {
                let single_game = if group.stats.is_single() {
                    group.games.iter().map(|(_, id)| *id).next()
                } else {
                    None
                };
                PreparedMove {
                    uci,
                    average_rating: group.stats.average_rating(),
                    average_opponent_rating: None,
                    game: single_game,
                    stats: group.stats,
                }
            })
            .collect();
        sort_by_key_and_truncate(&mut moves, limits.moves.unwrap_or(12), |m| {
            Reverse(m.stats.total())
        });

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
        let uci = Uci::Normal {
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

        let group = deserialized.groups.get(&uci).unwrap();
        assert_eq!(group.stats.draws, 1);
        assert_eq!(group.games[0], (1600 + 1700, game));
    }
}
