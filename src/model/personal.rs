use super::{Speed, Mode};
use std::io::{self, Read, Write};
use byteorder::{ReadBytesExt as _, WriteBytesExt as _};

struct Header {
    mode: Mode,
    speed: Speed,
    games: u8,
}

impl Header {
    fn read<R: Read>(reader: &mut R) -> io::Result<Header> {
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
}
