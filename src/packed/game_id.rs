use std::{fmt, fmt::Write, str::FromStr};

use arbitrary::{Arbitrary, Unstructured};
use bytes::{Buf, BufMut};
use thiserror::Error;

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GameId(u64);

impl GameId {
    pub const SIZE: usize = 6;

    /// Number of valid game ids (8 base 62 digits).
    pub const COUNT: u64 = 62_u64.pow(8);

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut bytes = [0u8; Self::SIZE];
        self.pack(&mut &mut bytes[..]);
        bytes
    }

    /// The partition of the game id when the id space is split into
    /// `partitions` equal ranges.
    pub fn partition(self, partitions: usize) -> usize {
        (u128::from(self.0) * partitions as u128 / u128::from(GameId::COUNT)) as usize
    }

    pub fn pack<B: BufMut>(&self, buf: &mut B) {
        buf.put_uint(self.0, Self::SIZE);
    }

    pub fn unpack<B: Buf>(buf: &mut B) -> Self {
        let mut bytes = [0; Self::SIZE];
        buf.copy_to_slice(&mut bytes);
        GameId::try_from(bytes).expect("valid game id")
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<Self, InvalidGameId> {
        if bytes.len() != 8 {
            return Err(InvalidGameId);
        }

        let mut base62 = 0;
        for c in bytes.iter().rev() {
            base62 = u64::from(match c {
                b'0'..=b'9' => c - b'0',
                b'A'..=b'Z' => c - b'A' + 10,
                b'a'..=b'z' => c - b'a' + 10 + 26,
                _ => return Err(InvalidGameId),
            }) + base62 * 62;
        }

        Ok(GameId(base62))
    }
}

#[derive(Error, Debug)]
#[error("invalid game id")]
pub struct InvalidGameId;

impl TryFrom<[u8; GameId::SIZE]> for GameId {
    type Error = InvalidGameId;

    fn try_from(bytes: [u8; GameId::SIZE]) -> Result<GameId, InvalidGameId> {
        let n = (&bytes[..]).get_uint(GameId::SIZE);
        if n < GameId::COUNT {
            Ok(GameId(n))
        } else {
            Err(InvalidGameId)
        }
    }
}

impl FromStr for GameId {
    type Err = InvalidGameId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_ascii(s.as_bytes())
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

impl fmt::Debug for GameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GameId({self})")
    }
}

impl Arbitrary<'_> for GameId {
    fn arbitrary(g: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(GameId(g.int_in_range(0..=GameId::COUNT - 1)?))
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_str_roundtrip() {
        arbtest(|u| {
            let game_id: GameId = u.arbitrary()?;

            assert_eq!(GameId::from_str(&game_id.to_string()).unwrap(), game_id);
            Ok(())
        });
    }

    #[test]
    fn test_pack_roundtrip() {
        arbtest(|u| {
            let game_id: GameId = u.arbitrary()?;

            let mut buf = Vec::new();
            game_id.pack(&mut buf);
            assert_eq!(GameId::unpack(&mut &buf[..]), game_id);
            Ok(())
        });
    }

    #[test]
    fn test_packed_ord() {
        arbtest(|u| {
            let left: GameId = u.arbitrary()?;
            let right: GameId = u.arbitrary()?;

            let mut left_buf = Vec::new();
            left.pack(&mut left_buf);

            let mut right_buf = Vec::new();
            right.pack(&mut right_buf);

            assert_eq!(left_buf.cmp(&right_buf), left.cmp(&right));
            Ok(())
        });
    }

    #[test]
    fn test_partition_ord() {
        arbtest(|u| {
            let left: GameId = u.arbitrary()?;
            let right: GameId = u.arbitrary()?;
            let partitions = u.int_in_range(1..=1 << 20)?;
            if left <= right {
                assert!(left.partition(partitions) <= right.partition(partitions));
            }
            Ok(())
        });
    }

    #[test]
    fn test_subpartition() {
        arbtest(|u| {
            let game_id: GameId = u.arbitrary()?;
            let coarse = u.int_in_range(1..=1 << 20)?;
            let sub = u.int_in_range(1..=1 << 10)?;
            assert_eq!(
                game_id.partition(coarse * sub) / sub,
                game_id.partition(coarse)
            );
            Ok(())
        });
    }
}
