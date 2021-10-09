use super::{
    read_uci, read_uint, write_uci, write_uint, ByMode, BySpeed, GameId, Mode, Speed, UserId,
};
use byteorder::{ByteOrder as _, LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use rustc_hash::FxHashMap;
use sha1::{Digest, Sha1};
use shakmaty::uci::Uci;
use shakmaty::{Color, Outcome};
use smallvec::{smallvec, SmallVec};
use std::cmp::max;
use std::io::{self, Read, Write};
use std::ops::AddAssign;

const MAX_GAMES: u64 = 15; // 4 bits

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

#[derive(Debug, Default)]
struct Stats {
    white: u64,
    draw: u64,
    black: u64,
}

impl From<Outcome> for Stats {
    fn from(outcome: Outcome) -> Stats {
        match outcome {
            Outcome::Decisive {
                winner: Color::White,
            } => Stats {
                white: 1,
                draw: 0,
                black: 0,
            },
            Outcome::Decisive {
                winner: Color::Black,
            } => Stats {
                white: 0,
                draw: 0,
                black: 1,
            },
            Outcome::Draw => Stats {
                white: 0,
                draw: 1,
                black: 1,
            },
        }
    }
}

impl AddAssign for Stats {
    fn add_assign(&mut self, rhs: Stats) {
        self.white += rhs.white;
        self.draw += rhs.draw;
        self.black += rhs.black;
    }
}

impl Stats {
    fn read<R: Read>(reader: &mut R) -> io::Result<Stats> {
        Ok(Stats {
            white: read_uint(reader)?,
            draw: read_uint(reader)?,
            black: read_uint(reader)?,
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.white)?;
        write_uint(writer, self.draw)?;
        write_uint(writer, self.black)
    }
}

#[derive(Default)]
struct Group {
    stats: Stats,
    games: SmallVec<[(u64, GameId); 1]>,
}

impl AddAssign for Group {
    fn add_assign(&mut self, rhs: Group) {
        self.stats += rhs.stats;
        self.games.extend(rhs.games);
    }
}

#[derive(Default)]
pub struct PersonalEntry {
    sub_entries: FxHashMap<Uci, BySpeed<ByMode<Group>>>,
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
        let mut sub_entry: BySpeed<ByMode<Group>> = Default::default();
        *sub_entry.by_speed_mut(speed).by_mode_mut(mode) = Group {
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

            loop {
                match Header::read(reader)? {
                    Header::Group {
                        speed,
                        mode,
                        num_games,
                    } => {
                        let stats = Stats::read(reader)?;
                        let mut games = SmallVec::with_capacity(num_games);
                        for _ in 0..num_games {
                            let game_idx = base_game_idx + read_uint(reader)?;
                            self.max_game_idx = max(self.max_game_idx, game_idx);
                            let game = GameId::read(reader)?;
                            games.push((game_idx, game));
                        }
                        let group = sub_entry.by_speed_mut(speed).by_mode_mut(mode);
                        *group += Group { stats, games };
                    }
                    Header::End => break,
                }
            }
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let discarded_game_idx = self.max_game_idx.saturating_sub(MAX_GAMES);

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

    pub fn with_zobrist(&self, zobrist: u128) -> PersonalKeyPrefix {
        PersonalKeyPrefix {
            prefix: self.base ^ zobrist,
        }
    }
}

#[derive(Debug)]
pub struct PersonalKeyPrefix {
    prefix: u128,
}

impl PersonalKeyPrefix {
    pub fn prefix(&self) -> [u8; 16] {
        let mut buf = [0; 16];
        LittleEndian::write_u128(&mut buf, self.prefix);
        buf
    }

    pub fn with_year(&self, year: u8) -> [u8; 17] {
        let mut buf = [0; 17];
        LittleEndian::write_u128(&mut buf, self.prefix);
        buf[16] = year;
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
