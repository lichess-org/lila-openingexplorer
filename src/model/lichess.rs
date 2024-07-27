use std::{
    array,
    cmp::{max, min, Reverse},
    str::FromStr,
};

use bytes::{Buf, BufMut};
use nohash_hasher::IntMap;
use shakmaty::{uci::UciMove, Outcome};
use thin_vec::{thin_vec, ThinVec};

use crate::{
    api::{LichessQueryFilter, Limits},
    model::{read_uint, write_uint, BySpeed, GameId, RawUciMove, Speed, Stats},
    util::{midpoint, sort_by_key_and_truncate},
};

const MAX_LICHESS_GAMES: usize = 8;
const MAX_TOP_GAMES: usize = 4; // <= MAX_LICHESS_GAMES

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RatingGroup {
    GroupLow,
    Group1000,
    Group1200,
    Group1400,
    Group1600,
    Group1800,
    Group2000,
    Group2200,
    Group2500,
    Group2800, // TODO: Tweak rating groups for better top game selection
    Group3200,
}

impl RatingGroup {
    pub const ALL: [RatingGroup; 11] = [
        RatingGroup::GroupLow,
        RatingGroup::Group1000,
        RatingGroup::Group1200,
        RatingGroup::Group1400,
        RatingGroup::Group1600,
        RatingGroup::Group1800,
        RatingGroup::Group2000,
        RatingGroup::Group2200,
        RatingGroup::Group2500,
        RatingGroup::Group2800,
        RatingGroup::Group3200,
    ];

    fn select_avg(avg: u16) -> RatingGroup {
        if avg < 1000 {
            RatingGroup::GroupLow
        } else if avg < 1200 {
            RatingGroup::Group1000
        } else if avg < 1400 {
            RatingGroup::Group1200
        } else if avg < 1600 {
            RatingGroup::Group1400
        } else if avg < 1800 {
            RatingGroup::Group1600
        } else if avg < 2000 {
            RatingGroup::Group1800
        } else if avg < 2200 {
            RatingGroup::Group2000
        } else if avg < 2500 {
            RatingGroup::Group2200
        } else if avg < 2800 {
            RatingGroup::Group2500
        } else {
            RatingGroup::Group3200
        }
    }

    fn select(mover_rating: u16, opponent_rating: u16) -> RatingGroup {
        RatingGroup::select_avg(midpoint(mover_rating, opponent_rating))
    }
}

impl FromStr for RatingGroup {
    type Err = <u16 as FromStr>::Err;

    fn from_str(s: &str) -> Result<RatingGroup, Self::Err> {
        Ok(RatingGroup::select_avg(s.parse()?))
    }
}

#[derive(Default, Debug)]
struct ByRatingGroup<T> {
    group_low: T,
    group_1000: T,
    group_1200: T,
    group_1400: T,
    group_1600: T,
    group_1800: T,
    group_2000: T,
    group_2200: T,
    group_2500: T,
    group_2800: T,
    group_3200: T,
}

impl<T> ByRatingGroup<T> {
    fn by_rating_group_mut(&mut self, rating_group: RatingGroup) -> &mut T {
        match rating_group {
            RatingGroup::GroupLow => &mut self.group_low,
            RatingGroup::Group1000 => &mut self.group_1000,
            RatingGroup::Group1200 => &mut self.group_1200,
            RatingGroup::Group1400 => &mut self.group_1400,
            RatingGroup::Group1600 => &mut self.group_1600,
            RatingGroup::Group1800 => &mut self.group_1800,
            RatingGroup::Group2000 => &mut self.group_2000,
            RatingGroup::Group2200 => &mut self.group_2200,
            RatingGroup::Group2500 => &mut self.group_2500,
            RatingGroup::Group2800 => &mut self.group_2800,
            RatingGroup::Group3200 => &mut self.group_3200,
        }
    }

    fn as_ref(&self) -> ByRatingGroup<&T> {
        ByRatingGroup {
            group_low: &self.group_low,
            group_1000: &self.group_1000,
            group_1200: &self.group_1200,
            group_1400: &self.group_1400,
            group_1600: &self.group_1600,
            group_1800: &self.group_1800,
            group_2000: &self.group_2000,
            group_2200: &self.group_2200,
            group_2500: &self.group_2500,
            group_2800: &self.group_2800,
            group_3200: &self.group_3200,
        }
    }

    fn zip_rating_group(self) -> ByRatingGroup<(RatingGroup, T)> {
        ByRatingGroup {
            group_low: (RatingGroup::GroupLow, self.group_low),
            group_1000: (RatingGroup::Group1000, self.group_1000),
            group_1200: (RatingGroup::Group1200, self.group_1200),
            group_1400: (RatingGroup::Group1400, self.group_1400),
            group_1600: (RatingGroup::Group1600, self.group_1600),
            group_1800: (RatingGroup::Group1800, self.group_1800),
            group_2000: (RatingGroup::Group2000, self.group_2000),
            group_2200: (RatingGroup::Group2200, self.group_2200),
            group_2500: (RatingGroup::Group2500, self.group_2500),
            group_2800: (RatingGroup::Group2800, self.group_2800),
            group_3200: (RatingGroup::Group3200, self.group_3200),
        }
    }
}

impl<T> IntoIterator for ByRatingGroup<T> {
    type Item = T;
    type IntoIter = array::IntoIter<T, 11>;

    fn into_iter(self) -> Self::IntoIter {
        [
            self.group_low,
            self.group_1000,
            self.group_1200,
            self.group_1400,
            self.group_1600,
            self.group_1800,
            self.group_2000,
            self.group_2200,
            self.group_2500,
            self.group_2800,
            self.group_3200,
        ]
        .into_iter()
    }
}

enum LichessHeader {
    Group {
        rating_group: RatingGroup,
        speed: Speed,
        num_games: usize,
    },
    End,
}

impl LichessHeader {
    fn read<B: Buf>(buf: &mut B) -> LichessHeader {
        let n = buf.get_u8();
        let speed = match n & 7 {
            0 => return LichessHeader::End,
            1 => Speed::UltraBullet,
            2 => Speed::Bullet,
            3 => Speed::Blitz,
            4 => Speed::Rapid,
            5 => Speed::Classical,
            6 => Speed::Correspondence,
            _ => panic!("invalid speed"),
        };
        let rating_group = match (n >> 3) & 15 {
            0 => RatingGroup::GroupLow,
            1 => RatingGroup::Group1000,
            2 => RatingGroup::Group1200,
            3 => RatingGroup::Group1400,
            4 => RatingGroup::Group1600,
            5 => RatingGroup::Group1800,
            6 => RatingGroup::Group2000,
            7 => RatingGroup::Group2200,
            8 => RatingGroup::Group2500,
            9 => RatingGroup::Group2800,
            10 => RatingGroup::Group3200,
            _ => panic!("invalid rating group"),
        };
        let single_game = (n >> 7) != 0;
        LichessHeader::Group {
            speed,
            rating_group,
            num_games: if single_game {
                1
            } else {
                read_uint(buf) as usize
            },
        }
    }

    fn write<B: BufMut>(&self, buf: &mut B) {
        match *self {
            LichessHeader::End => buf.put_u8(0),
            LichessHeader::Group {
                speed,
                rating_group,
                num_games,
            } => {
                let single_game = num_games == 1;
                buf.put_u8(
                    (match speed {
                        Speed::UltraBullet => 1,
                        Speed::Bullet => 2,
                        Speed::Blitz => 3,
                        Speed::Rapid => 4,
                        Speed::Classical => 5,
                        Speed::Correspondence => 6,
                    }) | (match rating_group {
                        RatingGroup::GroupLow => 0,
                        RatingGroup::Group1000 => 1,
                        RatingGroup::Group1200 => 2,
                        RatingGroup::Group1400 => 3,
                        RatingGroup::Group1600 => 4,
                        RatingGroup::Group1800 => 5,
                        RatingGroup::Group2000 => 6,
                        RatingGroup::Group2200 => 7,
                        RatingGroup::Group2500 => 8,
                        RatingGroup::Group2800 => 9,
                        RatingGroup::Group3200 => 10,
                    } << 3)
                        | (u8::from(single_game) << 7),
                );
                if !single_game {
                    write_uint(buf, num_games as u64);
                }
            }
        }
    }
}

#[derive(Default, Debug)]
pub struct LichessGroup {
    pub stats: Stats,
    pub games: ThinVec<(u64, GameId)>,
}

#[derive(Default, Debug)]
pub struct LichessEntry {
    sub_entries: IntMap<RawUciMove, BySpeed<ByRatingGroup<LichessGroup>>>,
    min_game_idx: Option<u64>,
    max_game_idx: Option<u64>,
}

impl LichessEntry {
    pub const SIZE_HINT: usize = 13;

    pub fn new_single(
        uci: UciMove,
        speed: Speed,
        game_id: GameId,
        outcome: Outcome,
        mover_rating: u16,
        opponent_rating: u16,
    ) -> LichessEntry {
        let mut sub_entry: BySpeed<ByRatingGroup<LichessGroup>> = Default::default();
        *sub_entry
            .by_speed_mut(speed)
            .by_rating_group_mut(RatingGroup::select(mover_rating, opponent_rating)) =
            LichessGroup {
                stats: Stats::new_single(outcome, mover_rating),
                games: thin_vec![(0, game_id)],
            };
        LichessEntry {
            sub_entries: [(RawUciMove::from(uci), sub_entry)].into_iter().collect(),
            min_game_idx: Some(0),
            max_game_idx: Some(0),
        }
    }

    pub fn extend_from_reader<B: Buf>(&mut self, buf: &mut B) {
        let base_game_idx = self.max_game_idx.map_or(0, |idx| idx + 1);

        while buf.has_remaining() {
            let uci = RawUciMove::read(buf);
            let sub_entry = self.sub_entries.entry(uci).or_default();

            while buf.has_remaining() {
                match LichessHeader::read(buf) {
                    LichessHeader::End => break,
                    LichessHeader::Group {
                        speed,
                        rating_group,
                        num_games,
                    } => {
                        let group = sub_entry
                            .by_speed_mut(speed)
                            .by_rating_group_mut(rating_group);
                        group.stats += &Stats::read(buf);
                        group.games.extend((0..num_games).map(|_| {
                            let game_idx = base_game_idx + read_uint(buf);
                            self.min_game_idx =
                                Some(min(self.min_game_idx.unwrap_or(u64::MAX), game_idx));
                            self.max_game_idx = Some(max(self.max_game_idx.unwrap_or(0), game_idx));
                            let game = GameId::read(buf);
                            (game_idx, game)
                        }));
                    }
                }
            }
        }
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        for (i, (uci, sub_entry)) in self.sub_entries.iter().enumerate() {
            if i > 0 {
                LichessHeader::End.write(buf);
            }

            uci.write(buf);

            for (speed, by_rating_group) in sub_entry.as_ref().zip_speed() {
                for (rating_group, group) in by_rating_group.as_ref().zip_rating_group() {
                    if !group.stats.is_empty() {
                        let num_games = min(group.games.len(), MAX_LICHESS_GAMES);
                        LichessHeader::Group {
                            speed,
                            rating_group,
                            num_games,
                        }
                        .write(buf);

                        group.stats.write(buf);

                        for (game_idx, game) in &group.games[group.games.len() - num_games..] {
                            write_uint(buf, *game_idx - self.min_game_idx.unwrap_or(0));
                            game.write(buf);
                        }
                    }
                }
            }
        }
    }

    pub fn total(&self, filter: &LichessQueryFilter) -> Stats {
        let mut stats = Stats::default();

        for sub_entry in self.sub_entries.values() {
            for (speed, group) in sub_entry.as_ref().zip_speed() {
                if filter.contains_speed(speed) {
                    for (rating_group, group) in group.as_ref().zip_rating_group() {
                        if filter.contains_rating_group(rating_group) {
                            stats += &group.stats;
                        }
                    }
                }
            }
        }

        stats
    }

    pub fn prepare(self, filter: &LichessQueryFilter, limits: &Limits) -> PreparedResponse {
        let mut total = Stats::default();
        let mut moves = Vec::with_capacity(self.sub_entries.len());
        let mut recent_games: Vec<(RatingGroup, Speed, u64, UciMove, GameId)> = Vec::new();

        for (uci, sub_entry) in self.sub_entries {
            let uci = UciMove::from(uci);

            let mut latest_game: Option<(u64, GameId)> = None;
            let mut stats = Stats::default();

            for (speed, group) in sub_entry.as_ref().zip_speed() {
                if filter.contains_speed(speed) {
                    for (rating_group, group) in group.as_ref().zip_rating_group() {
                        if filter.contains_rating_group(rating_group) {
                            stats += &group.stats;

                            for (idx, game) in group.games.iter().copied() {
                                if latest_game.map_or(true, |(latest_idx, _game)| latest_idx < idx)
                                {
                                    latest_game = Some((idx, game));
                                }
                            }

                            if limits.games_wanted() {
                                recent_games.extend(group.games.iter().copied().map(
                                    |(idx, game)| (rating_group, speed, idx, uci.clone(), game),
                                ));
                            }
                        }
                    }
                }
            }

            if !stats.is_empty() {
                total += &stats;

                moves.push(PreparedMove {
                    uci,
                    average_rating: stats.average_rating(),
                    average_opponent_rating: None,
                    performance: None,
                    game: latest_game.filter(|_| stats.is_single()).map(|(_, id)| id),
                    stats,
                });
            }
        }

        sort_by_key_and_truncate(&mut moves, limits.moves, |row| Reverse(row.stats.total()));

        // Split out top games from recent games.
        let mut top_games = if let Some(top_group) = filter.top_group() {
            let mut top_games: Vec<_> = recent_games
                .iter()
                .filter(|(rating_group, speed, _, _, _)| {
                    *rating_group >= top_group && *speed != Speed::Correspondence
                })
                .cloned()
                .collect();
            sort_by_key_and_truncate(
                &mut top_games,
                MAX_TOP_GAMES * 2,
                |(rating_group, _, idx, _, _)| (Reverse(*rating_group), Reverse(*idx)),
            );
            sort_by_key_and_truncate(&mut top_games, MAX_TOP_GAMES, |(_, _, idx, _, _)| {
                Reverse(*idx)
            });
            recent_games.retain(|(_, _, _, _, recent_game)| {
                !top_games
                    .iter()
                    .any(|(_, _, _, _, top_game)| recent_game == top_game)
            });
            top_games
        } else {
            Vec::new()
        };
        let valid_recent_games = MAX_LICHESS_GAMES - top_games.len();
        top_games.truncate(limits.top_games);

        // Prepare recent games.
        sort_by_key_and_truncate(
            &mut recent_games,
            min(valid_recent_games, limits.recent_games),
            |(_, _, idx, _, _)| Reverse(*idx),
        );

        PreparedResponse {
            total,
            moves,
            top_games: top_games
                .into_iter()
                .map(|(_, _, _, uci, game)| (uci, game))
                .collect(),
            recent_games: recent_games
                .into_iter()
                .map(|(_, _, _, uci, game)| (uci, game))
                .collect(),
        }
    }
}

#[derive(Debug)]
pub struct PreparedResponse {
    pub total: Stats,
    pub moves: Vec<PreparedMove>,
    pub recent_games: Vec<(UciMove, GameId)>,
    pub top_games: Vec<(UciMove, GameId)>,
}

#[derive(Debug)]
pub struct PreparedMove {
    pub uci: UciMove,
    pub stats: Stats,
    pub game: Option<GameId>,
    pub average_rating: Option<u16>,
    pub average_opponent_rating: Option<u16>,
    pub performance: Option<i32>,
}

#[cfg(test)]
mod tests {
    use shakmaty::{Color, Square};

    use super::*;

    #[test]
    fn test_lichess_entry() {
        // Roundtrip with a single entry.
        let uci_a = UciMove::Normal {
            from: Square::G1,
            to: Square::F3,
            promotion: None,
        };

        let a = LichessEntry::new_single(
            uci_a.clone(),
            Speed::Blitz,
            "aaaaaaaa".parse().unwrap(),
            Outcome::Draw,
            2000,
            2200,
        );

        let mut buf = Vec::new();
        a.write(&mut buf);
        assert_eq!(
            buf.len(),
            LichessEntry::SIZE_HINT,
            "optimized for single entries"
        );

        let mut deserialized = LichessEntry::default();
        deserialized.extend_from_reader(&mut &buf[..]);

        assert_eq!(deserialized.sub_entries.len(), 1);
        assert_eq!(deserialized.max_game_idx, Some(0));

        // Merge a second entry.
        let uci_b = UciMove::Normal {
            from: Square::D2,
            to: Square::D4,
            promotion: None,
        };

        let b = LichessEntry::new_single(
            uci_b.clone(),
            Speed::Blitz,
            "bbbbbbbb".parse().unwrap(),
            Outcome::Decisive {
                winner: Color::White,
            },
            2000,
            2200,
        );

        let mut buf = Vec::new();
        b.write(&mut buf);
        deserialized.extend_from_reader(&mut &buf[..]);

        assert_eq!(deserialized.sub_entries.len(), 2);
        assert_eq!(deserialized.max_game_idx, Some(1));

        // Roundtrip the combined entry.
        let mut buf = Vec::new();
        deserialized.write(&mut buf);
        let mut deserialized = LichessEntry::default();
        deserialized.extend_from_reader(&mut &buf[..]);

        assert_eq!(deserialized.sub_entries.len(), 2);
        assert_eq!(deserialized.max_game_idx, Some(1));

        // Run query.
        let res = deserialized.prepare(
            &LichessQueryFilter {
                speeds: None,
                ratings: Some([RatingGroup::Group2000].into()),
                since: None,
                until: None,
            },
            &Limits {
                recent_games: usize::MAX,
                top_games: usize::MAX,
                moves: Limits::default_moves(),
            },
        );
        assert_eq!(
            res.recent_games,
            &[
                (uci_b, "bbbbbbbb".parse().unwrap()),
                (uci_a, "aaaaaaaa".parse().unwrap()),
            ]
        );
    }
}
