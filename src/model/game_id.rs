use std::{
    fmt::{self, Write as _},
    str::FromStr,
};

use bytes::{Buf, BufMut};
use thiserror::Error;

#[derive(Error, Debug)]
#[error("invalid game id")]
pub struct InvalidGameId;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct GameId(u64);

impl GameId {
    pub const SIZE: usize = 6;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        self.write(&mut &mut buf[..]);
        buf
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        buf.put_uint_le(self.0, Self::SIZE);
    }

    pub fn read<B: Buf>(buf: &mut B) -> GameId {
        let n = buf.get_uint_le(Self::SIZE);
        assert!(n < 62u64.pow(8), "invalid game id");
        GameId(n)
    }
}

impl FromStr for GameId {
    type Err = InvalidGameId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 8 {
            return Err(InvalidGameId);
        }

        let mut n = 0;
        for c in s.bytes().rev() {
            n = u64::from(match c {
                b'0'..=b'9' => c - b'0',
                b'A'..=b'Z' => c - b'A' + 10,
                b'a'..=b'z' => c - b'a' + 10 + 26,
                _ => return Err(InvalidGameId),
            }) + n * 62;
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
    use quickcheck::{quickcheck, Arbitrary, Gen};

    use super::*;

    impl Arbitrary for GameId {
        fn arbitrary(g: &mut Gen) -> GameId {
            GameId(u64::arbitrary(g) % 62u64.pow(8))
        }
    }

    quickcheck! {
        fn test_game_id_roundtrip(game_id: GameId) -> bool {
            GameId::from_str(&game_id.to_string()).unwrap() == game_id
        }
    }
}
