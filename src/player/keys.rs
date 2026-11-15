//! Keys of the column families of the personal explorer. Every key of
//! player data starts with the `UserHash`, so that all data of a player is
//! one contiguous key range.
//!
//! ```text
//! status    uh                                  -> PlayerStatus
//! games     uh | PlayerGameIndex                -> PlayerGame
//! game_ids  uh | GameId                         -> PlayerGameIndex
//! warm      uh | gen | color | PositionKey      -> PlayerEntry
//! cold      uh | gen | color | bucket           -> ColdGroup
//! meta      name                                -> value (no player)
//! ```

use std::fmt;

use arbitrary::Arbitrary;
use shakmaty::Color;

use crate::{
    packed::{GameId, GameIndex, PositionKey, color_bits},
    player::UserHash,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PlayerGameIndex(pub u32);

impl PlayerGameIndex {
    pub const SIZE: usize = 4;

    pub fn next(self) -> PlayerGameIndex {
        PlayerGameIndex(self.0.checked_add(1).expect("game numbers exhausted"))
    }

    pub fn to_bytes(self) -> [u8; PlayerGameIndex::SIZE] {
        self.0.to_be_bytes()
    }

    pub fn from_bytes(bytes: [u8; PlayerGameIndex::SIZE]) -> PlayerGameIndex {
        PlayerGameIndex(u32::from_be_bytes(bytes))
    }
}

impl fmt::Display for PlayerGameIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<PlayerGameIndex> for GameIndex {
    fn from(index: PlayerGameIndex) -> GameIndex {
        GameIndex::new(u64::from(index.0)).expect("player game index fits")
    }
}

impl TryFrom<GameIndex> for PlayerGameIndex {
    type Error = std::num::TryFromIntError;

    fn try_from(index: GameIndex) -> Result<PlayerGameIndex, Self::Error> {
        u32::try_from(index.get()).map(PlayerGameIndex)
    }
}

/// Generation of the warm and cold index of a player. Rebuilds write the
/// next generation and then switch to it.
#[derive(Arbitrary, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(pub u32);

impl Generation {
    pub const SIZE: usize = 4;

    pub fn next(self) -> Generation {
        Generation(self.0.checked_add(1).expect("generations exhausted"))
    }

    pub fn to_bytes(self) -> [u8; Generation::SIZE] {
        self.0.to_be_bytes()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatusKey([u8; StatusKey::SIZE]);

impl AsRef<[u8]> for StatusKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl StatusKey {
    pub const SIZE: usize = UserHash::SIZE;

    pub fn new(uh: UserHash) -> StatusKey {
        StatusKey(*uh.as_bytes())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameKey([u8; GameKey::SIZE]);

impl AsRef<[u8]> for GameKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl GameKey {
    pub const SIZE: usize = UserHash::SIZE + PlayerGameIndex::SIZE;

    pub fn new(uh: UserHash, index: PlayerGameIndex) -> GameKey {
        let mut key = [0; GameKey::SIZE];
        key[..UserHash::SIZE].copy_from_slice(uh.as_bytes());
        key[UserHash::SIZE..].copy_from_slice(&index.to_bytes());
        GameKey(key)
    }

    pub fn index(key: &[u8]) -> PlayerGameIndex {
        PlayerGameIndex::from_bytes(
            key[UserHash::SIZE..GameKey::SIZE]
                .try_into()
                .expect("index"),
        )
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameIdKey([u8; GameIdKey::SIZE]);

impl AsRef<[u8]> for GameIdKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl GameIdKey {
    pub const SIZE: usize = UserHash::SIZE + GameId::SIZE;

    pub fn new(uh: UserHash, id: GameId) -> GameIdKey {
        let mut key = [0; GameIdKey::SIZE];
        key[..UserHash::SIZE].copy_from_slice(uh.as_bytes());
        key[UserHash::SIZE..].copy_from_slice(&id.to_bytes());
        GameIdKey(key)
    }
}

const INDEX_PREFIX_SIZE: usize = UserHash::SIZE + Generation::SIZE + 1;

fn index_prefix(uh: UserHash, generation: Generation, color: Color) -> [u8; INDEX_PREFIX_SIZE] {
    let mut key = [0; INDEX_PREFIX_SIZE];
    key[..UserHash::SIZE].copy_from_slice(uh.as_bytes());
    key[UserHash::SIZE..UserHash::SIZE + Generation::SIZE].copy_from_slice(&generation.to_bytes());
    key[INDEX_PREFIX_SIZE - 1] = color_bits(color);
    key
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WarmKey([u8; WarmKey::SIZE]);

impl AsRef<[u8]> for WarmKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl WarmKey {
    pub const SIZE: usize = INDEX_PREFIX_SIZE + PositionKey::SIZE;

    pub fn new(uh: UserHash, generation: Generation, color: Color, pos: &PositionKey) -> WarmKey {
        let mut key = [0; WarmKey::SIZE];
        key[..INDEX_PREFIX_SIZE].copy_from_slice(&index_prefix(uh, generation, color));
        key[INDEX_PREFIX_SIZE..].copy_from_slice(pos.as_bytes());
        WarmKey(key)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColdBucket(pub u32);

pub const MAX_COLD_BITS: u8 = 24;

impl ColdBucket {
    /// The bucket and the suffix (the 16 bits after the bucket bits, see
    /// `ColdGroup`) of a position.
    pub fn of(pos: &PositionKey, bits: u8) -> (ColdBucket, u16) {
        debug_assert!(bits <= MAX_COLD_BITS);
        let prefix = pos.prefix();
        let bucket = if bits == 0 {
            0
        } else {
            (prefix >> (64 - u32::from(bits))) as u32
        };
        let suffix = ((prefix << bits) >> 48) as u16;
        (ColdBucket(bucket), suffix)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColdGroupKey([u8; ColdGroupKey::SIZE]);

impl AsRef<[u8]> for ColdGroupKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl ColdGroupKey {
    pub const SIZE: usize = INDEX_PREFIX_SIZE + 4;

    pub fn new(
        uh: UserHash,
        generation: Generation,
        color: Color,
        bucket: ColdBucket,
    ) -> ColdGroupKey {
        let mut key = [0; ColdGroupKey::SIZE];
        key[..INDEX_PREFIX_SIZE].copy_from_slice(&index_prefix(uh, generation, color));
        key[INDEX_PREFIX_SIZE..].copy_from_slice(&bucket.0.to_be_bytes());
        ColdGroupKey(key)
    }
}

pub const MAX_KEY_SIZE: usize = WarmKey::SIZE;

const _: () = assert!(
    StatusKey::SIZE <= MAX_KEY_SIZE
        && GameKey::SIZE <= MAX_KEY_SIZE
        && GameIdKey::SIZE <= MAX_KEY_SIZE
        && ColdGroupKey::SIZE <= MAX_KEY_SIZE
);

impl UserHash {
    /// All keys of the player in any column family: `[uh, uh + 1)`, or for
    /// the largest hash up to `uh` and more `0xff` than any key has after
    /// it.
    pub fn range(self) -> (Vec<u8>, Vec<u8>) {
        let start = self.as_bytes().to_vec();
        let end = match self.to_u64().checked_add(1) {
            Some(next) => next.to_be_bytes().to_vec(),
            None => {
                let mut end = start.clone();
                end.resize(MAX_KEY_SIZE + 1, 0xff);
                end
            }
        };
        (start, end)
    }

    /// The warm or cold keys of generations `from..to` (or all from `from`).
    pub fn generations_range(self, from: Generation, to: Option<Generation>) -> (Vec<u8>, Vec<u8>) {
        let mut start = self.as_bytes().to_vec();
        start.extend_from_slice(&from.to_bytes());
        let end = match to {
            Some(to) => {
                let mut end = self.as_bytes().to_vec();
                end.extend_from_slice(&to.to_bytes());
                end
            }
            None => self.range().1,
        };
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cold_bucket() {
        let mut bytes = [0; PositionKey::SIZE];
        bytes[..8].copy_from_slice(&0xabcd_ef01_2345_6789u64.to_be_bytes());
        let pos = PositionKey::from_bytes(bytes);
        assert_eq!(ColdBucket::of(&pos, 0), (ColdBucket(0), 0xabcd));
        assert_eq!(ColdBucket::of(&pos, 4), (ColdBucket(0xa), 0xbcde));
        assert_eq!(ColdBucket::of(&pos, 16), (ColdBucket(0xabcd), 0xef01));
        assert_eq!(ColdBucket::of(&pos, 24), (ColdBucket(0xabcdef), 0x0123));
    }

    #[test]
    fn test_ranges() {
        let uh = UserHash::from_bytes(*b"\x12\x34\x56\x78\x9a\xbc\xde\xf0");
        let (start, end) = uh.range();
        let key = WarmKey::new(
            uh,
            Generation(7),
            Color::Black,
            &PositionKey::from_bytes([0xff; 12]),
        );
        assert!(start.as_slice() <= key.as_ref() && key.as_ref() < end.as_slice());
        let (start, end) = uh.generations_range(Generation(7), Some(Generation(8)));
        assert!(start.as_slice() <= key.as_ref() && key.as_ref() < end.as_slice());
        let (start, _) = uh.generations_range(Generation(8), None);
        assert!(key.as_ref() < start.as_slice());
        let max = UserHash::from_bytes([0xff; 8]);
        let (start, end) = max.range();
        let key = GameKey::new(max, PlayerGameIndex(u32::MAX));
        assert!(start.as_slice() <= key.as_ref() && key.as_ref() < end.as_slice());
        let key = WarmKey::new(
            max,
            Generation(u32::MAX),
            Color::Black,
            &PositionKey::from_bytes([0xff; 12]),
        );
        assert!(start.as_slice() <= key.as_ref() && key.as_ref() < end.as_slice());
    }
}
