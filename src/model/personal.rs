use super::{read_uci, read_uint, write_uci, write_uint, ByMode, BySpeed, GameId, Mode, Speed};
use byteorder::{ReadBytesExt as _, WriteBytesExt as _};
use shakmaty::uci::Uci;
use smallvec::SmallVec;
use std::cmp::max;
use std::collections::HashMap;
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
                1 => Speed::Ultrabullet,
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
                    Speed::Ultrabullet => 1,
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

struct Entry {
    sub_entries: HashMap<Uci, BySpeed<ByMode<Group>>>,
    max_game_idx: u64,
}

impl Entry {
    fn extend_from_reader<R: Read>(&mut self, reader: &mut R) -> io::Result<()> {
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

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
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
