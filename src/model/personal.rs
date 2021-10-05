use super::{read_uint, write_uint, ByMode, BySpeed, GameId, Mode, Speed};
use byteorder::{ReadBytesExt as _, WriteBytesExt as _};
use std::io::{self, Read, Write};

#[derive(Debug, Eq, PartialEq)]
struct Header {
    mode: Mode,
    speed: Speed,
    games: u8, // up to 15
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
            games: n >> 4,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(
            (if self.mode.is_rated() { 1 } else { 0 })
                | (match self.speed {
                    Speed::Ultrabullet => 0,
                    Speed::Bullet => 1,
                    Speed::Blitz => 2,
                    Speed::Rapid => 3,
                    Speed::Classical => 4,
                    Speed::Correspondence => 5,
                } << 1)
                | (self.games << 4),
        )
    }
}

#[derive(Debug)]
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

type Group = BySpeed<ByMode<(Stats, Vec<(u8, GameId)>)>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_header_roundtrip() {
        let header = Header {
            mode: Mode::Rated,
            speed: Speed::Correspondence,
            games: 15,
        };

        let mut writer = Cursor::new(Vec::new());
        header.write(&mut writer).unwrap();

        let mut reader = Cursor::new(writer.into_inner());
        assert_eq!(Header::read(&mut reader).unwrap(), header);
    }
}
