use super::{read_uint, write_uint, ByMode, BySpeed, GameId, Mode, Record, Speed};
use byteorder::{ReadBytesExt as _, WriteBytesExt as _};
use std::io::{self, Read, Write};
use std::cmp::min;

const MAX_GAMES: usize = 15; // 4 bits

#[derive(Debug, Eq, PartialEq)]
struct Header {
    mode: Mode,
    speed: Speed,
    num_games: usize,
}

impl Header {
    pub fn read<R: Read>(reader: &mut R) -> io::Result<Header> {
        let n = reader.read_u8()?;
        Ok(Header {
            mode: Mode::from_rated(n & 1 == 1),
            speed: match (n >> 1) & 7 {
                0 => Speed::Ultrabullet,
                1 => Speed::Bullet,
                2 => Speed::Blitz,
                3 => Speed::Rapid,
                4 => Speed::Classical,
                5 => Speed::Correspondence,
                _ => return Err(io::ErrorKind::InvalidData.into()),
            },
            num_games: usize::from(n >> 4),
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(
            self.mode.is_rated() as u8
                | (match self.speed {
                    Speed::Ultrabullet => 0,
                    Speed::Bullet => 1,
                    Speed::Blitz => 2,
                    Speed::Rapid => 3,
                    Speed::Classical => 4,
                    Speed::Correspondence => 5,
                } << 1)
                | ((self.num_games as u8) << 4),
        )
    }
}

#[derive(Debug, Default)]
struct Stats {
    white: u64,
    draw: u64,
    black: u64,
}

impl Stats {
    pub fn read<R: Read>(reader: &mut R) -> io::Result<Stats> {
        Ok(Stats {
            white: read_uint(reader)?,
            draw: read_uint(reader)?,
            black: read_uint(reader)?,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.white)?;
        write_uint(writer, self.draw)?;
        write_uint(writer, self.black)
    }
}

struct PersonalRecord {
    inner: BySpeed<ByMode<(Stats, Vec<GameId>)>>,
}

impl Record for PersonalRecord {
    fn read<R: Read>(reader: &mut R) -> io::Result<PersonalRecord> {

    }

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.inner.as_ref().try_map(|speed, by_mode| {
            by_mode.as_ref().try_map(|mode, (stats, games)| {
                let num_games = min(games.len(), MAX_GAMES);

                Header {
                    speed,
                    mode,
                    num_games,
                }.write(writer)?;

                stats.write(writer)?;

                for game in games.iter().take(num_games) {
                    game.write(writer)?;
                }

                Ok::<_, io::Error>(())
            })
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_header_roundtrip() {
        let header = Header {
            mode: Mode::Rated,
            speed: Speed::Correspondence,
            num_games: 15,
        };

        let mut writer = Cursor::new(Vec::new());
        header.write(&mut writer).unwrap();

        let mut reader = Cursor::new(writer.into_inner());
        assert_eq!(Header::read(&mut reader).unwrap(), header);
    }
}
