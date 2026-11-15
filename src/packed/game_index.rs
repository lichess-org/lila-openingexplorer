use std::fmt;

use arbitrary::{Arbitrary, Unstructured};
use bytes::{Buf, BufMut};

/// A game, by its row in the parquet input (counting from the first row of
/// the first input file). This is the key of the games column family, and
/// how entries and cold positions refer to games. Packed as 5 big-endian
/// bytes, so that the byte order is the numeric order.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameIndex(u64);

impl GameIndex {
    pub const SIZE: usize = 5;
    pub const BITS: u32 = 8 * GameIndex::SIZE as u32;
    pub const MAX: GameIndex = GameIndex((1 << GameIndex::BITS) - 1);

    pub fn new(index: u64) -> Option<GameIndex> {
        Some(GameIndex(index)).filter(|index| *index <= GameIndex::MAX)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn to_bytes(self) -> [u8; GameIndex::SIZE] {
        self.0.to_be_bytes()[8 - GameIndex::SIZE..]
            .try_into()
            .expect("5 bytes")
    }

    pub fn pack<B: BufMut>(&self, buf: &mut B) {
        buf.put_uint(self.0, GameIndex::SIZE);
    }

    pub fn unpack<B: Buf>(buf: &mut B) -> GameIndex {
        GameIndex(buf.get_uint(GameIndex::SIZE))
    }
}

impl fmt::Debug for GameIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GameIndex({})", self.0)
    }
}

impl fmt::Display for GameIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Arbitrary<'_> for GameIndex {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(GameIndex(u.int_in_range(0..=GameIndex::MAX.0)?))
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_pack_roundtrip() {
        arbtest(|u| {
            let index: GameIndex = u.arbitrary()?;
            let mut buf = Vec::new();
            index.pack(&mut buf);
            assert_eq!(buf, index.to_bytes());
            assert_eq!(GameIndex::unpack(&mut &buf[..]), index);
            Ok(())
        });
    }

    #[test]
    fn test_byte_order() {
        arbtest(|u| {
            let (a, b): (GameIndex, GameIndex) = u.arbitrary()?;
            assert_eq!(a.cmp(&b), a.to_bytes().cmp(&b.to_bytes()));
            Ok(())
        });
    }
}
