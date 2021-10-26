use std::{
    cmp::{max, min, Reverse},
    io::{self, Read, Write},
    ops::AddAssign,
    str::FromStr,
};

use byteorder::{ReadBytesExt as _, WriteBytesExt as _};
use rustc_hash::FxHashMap;
use shakmaty::{uci::Uci, Outcome};
use smallvec::{smallvec, SmallVec};

use crate::{
    api::LichessQueryFilter,
    model::{read_uci, read_uint, write_uci, write_uint, BySpeed, GameId, Speed, Stats},
};

const MAX_LICHESS_GAMES: u64 = 15;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum RatingGroup {
    GroupLow,
    Group1600,
    Group1800,
    Group2000,
    Group2200,
    Group2500,
    Group2800,
    Group3200,
}

impl RatingGroup {
    const ALL: [RatingGroup; 8] = [
        RatingGroup::GroupLow,
        RatingGroup::Group1600,
        RatingGroup::Group1800,
        RatingGroup::Group2000,
        RatingGroup::Group2200,
        RatingGroup::Group2500,
        RatingGroup::Group2800,
        RatingGroup::Group3200,
    ];

    fn select_avg(avg: u16) -> RatingGroup {
        if avg < 1600 {
            RatingGroup::GroupLow
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
        RatingGroup::select_avg(mover_rating / 2 + opponent_rating / 2)
    }
}

impl FromStr for RatingGroup {
    type Err = <u16 as FromStr>::Err;

    fn from_str(s: &str) -> Result<RatingGroup, Self::Err> {
        Ok(RatingGroup::select_avg(s.parse()?))
    }
}

#[derive(Default)]
struct ByRatingGroup<T> {
    group_low: T,
    group_1600: T,
    group_1800: T,
    group_2000: T,
    group_2200: T,
    group_2500: T,
    group_2800: T,
    group_3200: T,
}

impl<T> ByRatingGroup<T> {
    fn by_rating_group(&self, rating_group: RatingGroup) -> &T {
        match rating_group {
            RatingGroup::GroupLow => &self.group_low,
            RatingGroup::Group1600 => &self.group_1600,
            RatingGroup::Group1800 => &self.group_1800,
            RatingGroup::Group2000 => &self.group_2000,
            RatingGroup::Group2200 => &self.group_2200,
            RatingGroup::Group2500 => &self.group_2500,
            RatingGroup::Group2800 => &self.group_2800,
            RatingGroup::Group3200 => &self.group_3200,
        }
    }

    fn by_rating_group_mut(&mut self, rating_group: RatingGroup) -> &mut T {
        match rating_group {
            RatingGroup::GroupLow => &mut self.group_low,
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
            group_1600: &self.group_1600,
            group_1800: &self.group_1800,
            group_2000: &self.group_2000,
            group_2200: &self.group_2200,
            group_2500: &self.group_2500,
            group_2800: &self.group_2800,
            group_3200: &self.group_3200,
        }
    }

    fn try_map<U, E, F>(self, mut f: F) -> Result<ByRatingGroup<U>, E>
    where
        F: FnMut(RatingGroup, T) -> Result<U, E>,
    {
        Ok(ByRatingGroup {
            group_low: f(RatingGroup::GroupLow, self.group_low)?,
            group_1600: f(RatingGroup::Group1600, self.group_1600)?,
            group_1800: f(RatingGroup::Group1800, self.group_1800)?,
            group_2000: f(RatingGroup::Group2000, self.group_2000)?,
            group_2200: f(RatingGroup::Group2200, self.group_2200)?,
            group_2500: f(RatingGroup::Group2500, self.group_2500)?,
            group_2800: f(RatingGroup::Group2800, self.group_2800)?,
            group_3200: f(RatingGroup::Group3200, self.group_3200)?,
        })
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
    fn read<R: Read>(reader: &mut R) -> io::Result<LichessHeader> {
        let n = reader.read_u8()?;
        let speed = match n & 7 {
            0 => return Ok(LichessHeader::End),
            1 => Speed::UltraBullet,
            2 => Speed::Bullet,
            3 => Speed::Blitz,
            4 => Speed::Rapid,
            5 => Speed::Classical,
            6 => Speed::Correspondence,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let rating_group = match (n >> 3) & 7 {
            0 => RatingGroup::GroupLow,
            1 => RatingGroup::Group1600,
            2 => RatingGroup::Group1800,
            3 => RatingGroup::Group2000,
            4 => RatingGroup::Group2200,
            5 => RatingGroup::Group2500,
            6 => RatingGroup::Group2800,
            7 => RatingGroup::Group3200,
            _ => unreachable!(),
        };
        let at_least_num_games = usize::from(n >> 6);
        Ok(LichessHeader::Group {
            speed,
            rating_group,
            num_games: if at_least_num_games >= 3 {
                usize::from(reader.read_u8()?)
            } else {
                at_least_num_games
            },
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match *self {
            LichessHeader::End => writer.write_u8(0),
            LichessHeader::Group {
                speed,
                rating_group,
                num_games,
            } => {
                writer.write_u8(
                    (match speed {
                        Speed::UltraBullet => 1,
                        Speed::Bullet => 2,
                        Speed::Blitz => 3,
                        Speed::Rapid => 4,
                        Speed::Classical => 5,
                        Speed::Correspondence => 6,
                    }) | (match rating_group {
                        RatingGroup::GroupLow => 0,
                        RatingGroup::Group1600 => 1,
                        RatingGroup::Group1800 => 2,
                        RatingGroup::Group2000 => 3,
                        RatingGroup::Group2200 => 4,
                        RatingGroup::Group2500 => 5,
                        RatingGroup::Group2800 => 6,
                        RatingGroup::Group3200 => 7,
                    } << 3)
                        | ((min(3, num_games) as u8) << 6),
                )?;
                if num_games >= 3 {
                    write_uint(writer, num_games as u64)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Default, Debug)]
pub struct LichessGroup {
    pub stats: Stats,
    pub games: SmallVec<[(u64, GameId); 1]>,
}

impl AddAssign for LichessGroup {
    fn add_assign(&mut self, rhs: LichessGroup) {
        self.stats += rhs.stats;
        self.games.extend(rhs.games);
    }
}

#[derive(Default)]
pub struct LichessEntry {
    sub_entries: FxHashMap<Uci, BySpeed<ByRatingGroup<LichessGroup>>>,
    max_game_idx: u64,
}

impl LichessEntry {
    pub const SIZE_HINT: usize = 14;

    pub fn new_single(
        uci: Uci,
        speed: Speed,
        game_id: GameId,
        outcome: Outcome,
        mover_rating: u16,
        opponent_rating: u16,
    ) -> LichessEntry {
        let rating_group = RatingGroup::select(mover_rating, opponent_rating);
        let mut sub_entry: BySpeed<ByRatingGroup<LichessGroup>> = Default::default();
        *sub_entry
            .by_speed_mut(speed)
            .by_rating_group_mut(rating_group) = LichessGroup {
            stats: Stats::new_single(outcome, mover_rating),
            games: smallvec![(0, game_id)],
        };
        let mut sub_entries = FxHashMap::with_capacity_and_hasher(1, Default::default());
        sub_entries.insert(uci, sub_entry);
        LichessEntry {
            sub_entries,
            max_game_idx: 0,
        }
    }

    pub fn extend_from_reader<R: Read>(&mut self, reader: &mut R) -> io::Result<()> {
        loop {
            let uci = match read_uci(reader) {
                Ok(uci) => uci,
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(err) => return Err(err),
            };

            let sub_entry = self.sub_entries.entry(uci).or_default();

            let base_game_idx = self.max_game_idx + 1;

            while let LichessHeader::Group {
                speed,
                rating_group,
                num_games,
            } = LichessHeader::read(reader)?
            {
                let stats = Stats::read(reader)?;
                let mut games = SmallVec::with_capacity(num_games);
                for _ in 0..num_games {
                    let game_idx = base_game_idx + read_uint(reader)?;
                    self.max_game_idx = max(self.max_game_idx, game_idx);
                    let game = GameId::read(reader)?;
                    games.push((game_idx, game));
                }
                let group = sub_entry
                    .by_speed_mut(speed)
                    .by_rating_group_mut(rating_group);
                *group += LichessGroup { stats, games };
            }
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let discarded_game_idx = self.max_game_idx.saturating_sub(MAX_LICHESS_GAMES);

        for (uci, sub_entry) in &self.sub_entries {
            write_uci(writer, uci)?;

            sub_entry.as_ref().try_map(|speed, by_rating_group| {
                by_rating_group.as_ref().try_map(|rating_group, group| {
                    let num_games = if group.games.len() == 1 {
                        1
                    } else {
                        group
                            .games
                            .iter()
                            .filter(|(game_idx, _)| *game_idx > discarded_game_idx)
                            .count()
                    };

                    if num_games > 0 || !group.stats.is_empty() {
                        LichessHeader::Group {
                            speed,
                            rating_group,
                            num_games,
                        }
                        .write(writer)?;

                        group.stats.write(writer)?;

                        for (game_idx, game) in &group.games {
                            if *game_idx > discarded_game_idx || group.games.len() == 1 {
                                write_uint(writer, *game_idx)?;
                                game.write(writer)?;
                            }
                        }
                    }

                    Ok::<_, io::Error>(())
                })
            })?;

            LichessHeader::End.write(writer)?;
        }

        Ok(())
    }

    pub fn prepare(self, filter: &LichessQueryFilter) -> PreparedResponse {
        let mut total = Stats::default();
        let mut moves = Vec::with_capacity(self.sub_entries.len());
        let mut recent_games: Vec<(u64, Uci, GameId)> = Vec::new();
        let mut top_games: Vec<(Uci, GameId)> = Vec::new();

        for (uci, sub_entry) in self.sub_entries {
            let mut latest_game: Option<(u64, GameId)> = None;
            let mut stats = Stats::default();
            let mut skipped_rating_group = false;

            for rating_group in RatingGroup::ALL.into_iter().rev() {
                if filter.contains_rating_group(rating_group) {
                    for speed in Speed::ALL.into_iter().rev() {
                        if filter.contains_speed(speed) {
                            let group = sub_entry.by_speed(speed).by_rating_group(rating_group);
                            stats += group.stats.to_owned();

                            for (idx, game) in group.games.iter().copied() {
                                if latest_game.map_or(true, |(latest_idx, _game)| latest_idx < idx)
                                {
                                    latest_game = Some((idx, game));
                                }
                            }

                            if !skipped_rating_group
                                && top_games.len() < 4
                                && rating_group >= RatingGroup::Group2000
                            {
                                top_games.extend(
                                    group
                                        .games
                                        .iter()
                                        .copied()
                                        .map(|(_, game)| (uci.to_owned(), game)),
                                );
                            } else {
                                recent_games.extend(
                                    group
                                        .games
                                        .iter()
                                        .copied()
                                        .map(|(idx, game)| (idx, uci.to_owned(), game)),
                                );
                            }
                        }
                    }
                } else {
                    skipped_rating_group = true;
                }
            }

            if !stats.is_empty() || latest_game.is_some() {
                moves.push(PreparedMove {
                    uci,
                    stats: stats.clone(),
                    average_rating: stats.average_rating(),
                    average_opponent_rating: None,
                    game: latest_game.filter(|_| stats.is_single()).map(|(_, id)| id),
                });
            }

            total += stats;
        }

        top_games.truncate(MAX_LICHESS_GAMES as usize);

        moves.sort_by_key(|row| Reverse(row.stats.total()));
        recent_games.sort_by_key(|(idx, _, _)| Reverse(*idx));

        PreparedResponse {
            total,
            moves,
            recent_games: recent_games
                .into_iter()
                .map(|(_, uci, game)| (uci, game))
                .take((MAX_LICHESS_GAMES as usize).saturating_sub(top_games.len()))
                .collect(),
            top_games,
        }
    }
}

#[derive(Debug)]
pub struct PreparedResponse {
    pub total: Stats,
    pub moves: Vec<PreparedMove>,
    pub recent_games: Vec<(Uci, GameId)>,
    pub top_games: Vec<(Uci, GameId)>,
}

#[derive(Debug)]
pub struct PreparedMove {
    pub uci: Uci,
    pub stats: Stats,
    pub game: Option<GameId>,
    pub average_rating: Option<u64>,
    pub average_opponent_rating: Option<u64>,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use shakmaty::Square;

    use super::*;

    #[test]
    fn test_lichess_entry() {
        let uci = Uci::Normal {
            from: Square::G1,
            to: Square::F3,
            promotion: None,
        };

        let a = LichessEntry::new_single(
            uci,
            Speed::Blitz,
            "99999999".parse().unwrap(),
            Outcome::Draw,
            2000,
            2200,
        );

        let mut cursor = Cursor::new(Vec::new());
        a.write(&mut cursor).unwrap();
        assert_eq!(
            cursor.position() as usize,
            LichessEntry::SIZE_HINT,
            "optimized for single entries"
        );

        let mut deserialized = LichessEntry::default();
        deserialized
            .extend_from_reader(&mut Cursor::new(cursor.into_inner()))
            .unwrap();

        assert_eq!(deserialized.sub_entries.len(), 1);
    }
}
