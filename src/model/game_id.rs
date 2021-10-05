use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use std::fmt::{self, Write as _};
use std::io;
use std::io::{Read, Write};
use std::str::FromStr;

#[derive(Debug)]
pub struct InvalidGameId;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GameId(u64);

impl GameId {
    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u48::<LittleEndian>(self.0)
    }

    pub fn read<R: Read>(&self, reader: &mut R) -> io::Result<GameId> {
        let n = reader.read_u48::<LittleEndian>()?;
        if n < 62u64.pow(8) {
            Ok(GameId(n))
        } else {
            Err(io::ErrorKind::InvalidData.into())
        }
    }
}

impl FromStr for GameId {
    type Err = InvalidGameId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if dbg!(s).len() != 8 {
            return Err(InvalidGameId);
        }

        let mut n = 0;
        for c in s.bytes().rev() {
            n = match c {
                b'0'..=b'9' => c - b'0',
                b'A'..=b'Z' => c - b'A' + 10,
                b'a'..=b'z' => c - b'a' + 10 + 26,
                _ => return Err(InvalidGameId),
            } as u64
                + n * 62;
        }

        Ok(GameId(n))
    }
}

impl fmt::Display for GameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut n = self.0;
        for _ in 0..8 {
            let rem = n % 62;
            f.write_char(char::from(if rem >= 10 + 26 {
                (rem - (10 + 26)) as u8 + b'a'
            } else if rem >= 10 {
                (rem - 10) as u8 + b'A'
            } else {
                rem as u8 + b'0'
            }))?;
            n /= 62;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{quickcheck, Arbitrary, Gen};

    impl Arbitrary for GameId {
        fn arbitrary(g: &mut Gen) -> Self {
            GameId(u64::arbitrary(g) % 62u64.pow(8))
        }
    }

    quickcheck! {
        fn game_id_roundtrip(game_id: GameId) -> bool {
            GameId::from_str(&game_id.to_string()).unwrap() == game_id
        }
    }
}
