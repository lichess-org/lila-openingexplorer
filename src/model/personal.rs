use crate::{
    api::PersonalQueryFilter,
    model::{
        read_uci, read_uint, write_uci, write_uint, AnnoLichess, ByMode, BySpeed, GameId, Mode,
        Speed, UserId,
    },
};
use byteorder::{ByteOrder as _, LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use rustc_hash::FxHashMap;
use serde::Serialize;
use sha1::{Digest, Sha1};
use shakmaty::{
    san::{San, SanPlus},
    uci::Uci,
    variant::{Variant, VariantPosition},
    Color, Outcome,
};
use smallvec::{smallvec, SmallVec};
use std::{
    cmp::{max, Reverse},
    io::{self, Read, Write},
    ops::AddAssign,
    time::{Duration, SystemTime},
};

const MAX_PERSONAL_GAMES: u64 = 15; // 4 bits

#[derive(Debug, Eq, PartialEq)]
enum Header {
    Group {
        mode: Mode,
        speed: Speed,
        num_games: usize,
    },
    End,
}

impl Header {
    fn read<R: Read>(reader: &mut R) -> io::Result<Header> {
        let n = reader.read_u8()?;
        Ok(Header::Group {
            speed: match n & 7 {
                0 => return Ok(Header::End),
                1 => Speed::UltraBullet,
                2 => Speed::Bullet,
                3 => Speed::Blitz,
                4 => Speed::Rapid,
                5 => Speed::Classical,
                6 => Speed::Correspondence,
                _ => return Err(io::ErrorKind::InvalidData.into()),
            },
            mode: Mode::from_rated((n >> 3) & 1 == 1),
            num_games: usize::from(n >> 4),
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(match *self {
            Header::End => 0,
            Header::Group {
                mode,
                speed,
                num_games,
            } => {
                (match speed {
                    Speed::UltraBullet => 1,
                    Speed::Bullet => 2,
                    Speed::Blitz => 3,
                    Speed::Rapid => 4,
                    Speed::Classical => 5,
                    Speed::Correspondence => 6,
                }) | ((mode.is_rated() as u8) << 3)
                    | ((num_games as u8) << 4)
            }
        })
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Stats {
    white: u64,
    draws: u64,
    black: u64,
}

impl From<Outcome> for Stats {
    fn from(outcome: Outcome) -> Stats {
        Stats {
            white: if outcome.winner() == Some(Color::White) {
                1
            } else {
                0
            },
            black: if outcome.winner() == Some(Color::Black) {
                1
            } else {
                0
            },
            draws: if outcome.winner().is_none() { 1 } else { 0 },
        }
    }
}

impl AddAssign for Stats {
    fn add_assign(&mut self, rhs: Stats) {
        self.white += rhs.white;
        self.draws += rhs.draws;
        self.black += rhs.black;
    }
}

impl Stats {
    pub fn total(&self) -> u64 {
        self.white + self.draws + self.black
    }

    fn read<R: Read>(reader: &mut R) -> io::Result<Stats> {
        Ok(Stats {
            white: read_uint(reader)?,
            draws: read_uint(reader)?,
            black: read_uint(reader)?,
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.white)?;
        write_uint(writer, self.draws)?;
        write_uint(writer, self.black)
    }
}

#[derive(Default, Debug, Clone)]
pub struct PersonalGroup {
    pub stats: Stats,
    pub games: SmallVec<[(u64, GameId); 1]>,
}

impl AddAssign for PersonalGroup {
    fn add_assign(&mut self, rhs: PersonalGroup) {
        self.stats += rhs.stats;
        self.games.extend(rhs.games);
    }
}

#[derive(Default, Debug)]
pub struct PersonalEntry {
    pub sub_entries: FxHashMap<Uci, BySpeed<ByMode<PersonalGroup>>>,
    max_game_idx: u64,
}

impl PersonalEntry {
    pub fn new_single(
        uci: Uci,
        speed: Speed,
        mode: Mode,
        game_id: GameId,
        outcome: Outcome,
    ) -> PersonalEntry {
        let mut sub_entry: BySpeed<ByMode<PersonalGroup>> = Default::default();
        *sub_entry.by_speed_mut(speed).by_mode_mut(mode) = PersonalGroup {
            stats: outcome.into(),
            games: smallvec![(0, game_id)],
        };
        let mut sub_entries = FxHashMap::with_capacity_and_hasher(1, Default::default());
        sub_entries.insert(uci, sub_entry);

        PersonalEntry {
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

            while let Header::Group {
                speed,
                mode,
                num_games,
            } = Header::read(reader)?
            {
                let stats = Stats::read(reader)?;
                let mut games = SmallVec::with_capacity(num_games);
                for _ in 0..num_games {
                    let game_idx = base_game_idx + read_uint(reader)?;
                    self.max_game_idx = max(self.max_game_idx, game_idx);
                    let game = GameId::read(reader)?;
                    games.push((game_idx, game));
                }
                let group = sub_entry.by_speed_mut(speed).by_mode_mut(mode);
                *group += PersonalGroup { stats, games };
            }
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let discarded_game_idx = self.max_game_idx.saturating_sub(MAX_PERSONAL_GAMES);

        for (uci, sub_entry) in &self.sub_entries {
            write_uci(writer, uci)?;

            sub_entry.as_ref().try_map(|speed, by_mode| {
                by_mode.as_ref().try_map(|mode, group| {
                    let num_games = if group.games.len() == 1 {
                        1
                    } else {
                        group
                            .games
                            .iter()
                            .filter(|(game_idx, _)| *game_idx > discarded_game_idx)
                            .count()
                    };

                    Header::Group {
                        speed,
                        mode,
                        num_games,
                    }
                    .write(writer)?;

                    group.stats.write(writer)?;

                    for (game_idx, game) in group.games.iter() {
                        if *game_idx > discarded_game_idx || group.games.len() == 1 {
                            write_uint(writer, *game_idx)?;
                            game.write(writer)?;
                        }
                    }

                    Ok::<_, io::Error>(())
                })
            })?;

            Header::End.write(writer)?;
        }

        Ok(())
    }

    pub fn prepare(
        self,
        pos: VariantPosition,
        filter: PersonalQueryFilter,
    ) -> FilteredPersonalEntry {
        let mut total = Stats::default();
        let mut moves = Vec::with_capacity(self.sub_entries.len());
        let mut recent_games: Vec<(u64, Uci, GameId)> = Vec::new();

        for (uci, sub_entry) in self.sub_entries {
            let san = uci.to_move(&pos).map_or(
                SanPlus {
                    san: San::Null,
                    suffix: None,
                },
                |m| SanPlus::from_move(pos.clone(), &m),
            );

            let mut latest_game: Option<(u64, GameId)> = None;
            let mut stats = Stats::default();

            for speed in Speed::ALL {
                if filter
                    .speeds
                    .as_ref()
                    .map_or(true, |speeds| speeds.contains(&speed))
                {
                    for mode in Mode::ALL {
                        if filter
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
            }

            total += stats.clone();

            moves.push(FilteredPersonalMoveRow {
                uci,
                san,
                stats,
                game: latest_game.map(|(_, id)| id),
            });
        }

        moves.sort_by_key(|row| Reverse(row.stats.total()));
        recent_games.sort_by_key(|(idx, _, _)| Reverse(*idx));

        FilteredPersonalEntry {
            total,
            moves,
            recent_games: recent_games
                .into_iter()
                .map(|(_, uci, game)| (uci, game))
                .take(MAX_PERSONAL_GAMES as usize)
                .collect(),
        }
    }
}

pub struct FilteredPersonalEntry {
    pub total: Stats,
    pub moves: Vec<FilteredPersonalMoveRow>,
    pub recent_games: Vec<(Uci, GameId)>,
}

pub struct FilteredPersonalMoveRow {
    pub uci: Uci,
    pub san: SanPlus,
    pub stats: Stats,
    pub game: Option<GameId>,
}

#[derive(Debug)]
pub struct PersonalKeyBuilder {
    base: u128,
}

impl PersonalKeyBuilder {
    pub fn with_user_pov(user: &UserId, color: Color) -> PersonalKeyBuilder {
        let mut hash = Sha1::new();
        hash.update(color.fold(b"w", b"b"));
        hash.update(user.as_str());
        let buf = hash.finalize();
        PersonalKeyBuilder {
            base: LittleEndian::read_u128(buf.as_slice()),
        }
    }

    pub fn with_zobrist(&self, variant: Variant, zobrist: u128) -> PersonalKeyPrefix {
        PersonalKeyPrefix {
            prefix: self.base
                ^ zobrist
                ^ (match variant {
                    Variant::Chess => 0,
                    Variant::Antichess => 0x44782fce075483666c81899cb65921c9,
                    Variant::Atomic => 0x66ccbd680f655d562689ca333c5e2a42,
                    Variant::Crazyhouse => 0x9d04db38ca4d923d82ff24eb9530e986,
                    Variant::Horde => 0xc29dfb1076aa15186effd0d34cc60737,
                    Variant::KingOfTheHill => 0xdfb25d5df41fc5961e61f6b4ba613fbe,
                    Variant::RacingKings => 0x8e72f94307f96710b3910cf7e5808e0d,
                    Variant::ThreeCheck => 0xd19242bae967b40e7856bd1c71aa4220,
                }),
        }
    }
}

#[derive(Debug)]
pub struct PersonalKeyPrefix {
    prefix: u128,
}

impl PersonalKeyPrefix {
    pub fn with_year(&self, AnnoLichess(year): AnnoLichess) -> PersonalKey {
        let mut buf = [0; 17];
        LittleEndian::write_u128(&mut buf, self.prefix);
        buf[16] = year;
        PersonalKey(buf)
    }
}

#[derive(Debug)]
pub struct PersonalKey([u8; 17]);

impl PersonalKey {
    pub fn into_bytes(self) -> [u8; 17] {
        self.0
    }
}

pub struct PersonalStatus {
    pub latest_created_at: u64,
    pub revisit_ongoing_created_at: Option<u64>,
    pub indexed_at: SystemTime,
}

impl Default for PersonalStatus {
    fn default() -> PersonalStatus {
        PersonalStatus {
            latest_created_at: 0,
            revisit_ongoing_created_at: None,
            indexed_at: SystemTime::UNIX_EPOCH,
        }
    }
}

impl PersonalStatus {
    pub fn maybe_revisit_ongoing(&mut self) -> Option<u64> {
        if SystemTime::now()
            .duration_since(self.indexed_at)
            .unwrap_or_default()
            > Duration::from_secs(24 * 60 * 60)
        {
            self.revisit_ongoing_created_at.take()
        } else {
            None
        }
    }

    pub fn maybe_index(&self) -> Option<u64> {
        SystemTime::now()
            .duration_since(self.indexed_at)
            .map_or(false, |cooldown| cooldown > Duration::from_secs(60))
            .then(|| self.latest_created_at)
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<PersonalStatus> {
        Ok(PersonalStatus {
            latest_created_at: read_uint(reader)?,
            revisit_ongoing_created_at: Some(read_uint(reader)?).filter(|t| *t != 0),
            indexed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(read_uint(reader)?),
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.latest_created_at)?;
        write_uint(writer, self.revisit_ongoing_created_at.unwrap_or(0))?;
        write_uint(
            writer,
            self.indexed_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("duration since unix epoch")
                .as_secs(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Square;
    use std::io::Cursor;

    #[test]
    fn test_header_roundtrip() {
        let headers = [
            Header::Group {
                mode: Mode::Rated,
                speed: Speed::Correspondence,
                num_games: 15,
            },
            Header::End,
        ];

        let mut writer = Cursor::new(Vec::new());
        for header in &headers {
            header.write(&mut writer).unwrap();
        }

        let mut reader = Cursor::new(writer.into_inner());
        for header in headers {
            assert_eq!(Header::read(&mut reader).unwrap(), header);
        }
    }

    #[test]
    fn test_merge_personal() {
        let uci = Uci::Normal {
            from: Square::E2,
            to: Square::E4,
            promotion: None,
        };

        let a = PersonalEntry::new_single(
            uci.clone(),
            Speed::Bullet,
            Mode::Rated,
            "12345678".parse().unwrap(),
            Outcome::Decisive {
                winner: Color::White,
            },
        );

        let b = PersonalEntry::new_single(
            uci.clone(),
            Speed::Bullet,
            Mode::Rated,
            "87654321".parse().unwrap(),
            Outcome::Decisive {
                winner: Color::Black,
            },
        );

        let mut deserialized = PersonalEntry::default();

        let mut cursor = Cursor::new(Vec::new());
        a.write(&mut cursor).unwrap();
        deserialized
            .extend_from_reader(&mut Cursor::new(cursor.into_inner()))
            .unwrap();

        let mut cursor = Cursor::new(Vec::new());
        b.write(&mut cursor).unwrap();
        deserialized
            .extend_from_reader(&mut Cursor::new(cursor.into_inner()))
            .unwrap();

        let group = deserialized
            .sub_entries
            .get(&uci)
            .unwrap()
            .by_speed(Speed::Bullet)
            .by_mode(Mode::Rated);
        assert_eq!(group.stats.white, 1);
        assert_eq!(group.stats.black, 1);
    }
}
