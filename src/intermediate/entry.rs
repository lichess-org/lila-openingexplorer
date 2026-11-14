use std::fmt;

use bytes::{Buf, BufMut};
use shakmaty::{KnownOutcome, packed::PackedUciMove};

use crate::packed::{
    CellGroupHeader, CellKey, GameIndex, GroupHeader, Month, PositionKey, Rating, outcome_bits,
    outcome_from_bits,
};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct IntermediateEntry(pub [u8; IntermediateEntry::SIZE]);

impl IntermediateEntry {
    pub const SIZE: usize = 24;
    pub const KEY_SIZE: usize = PositionKey::SIZE;
    /// Bits for the game index, packed into 5 bytes with the outcome.
    pub const GAME_INDEX_BITS: u32 = 38;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hash: PositionKey,
        month: Month,
        m: PackedUciMove,
        header: GroupHeader,
        rating: Rating,
        outcome: KnownOutcome,
        game: GameIndex,
    ) -> IntermediateEntry {
        assert!(
            game.get() < 1 << Self::GAME_INDEX_BITS,
            "game index {game} exceeds {} bits",
            Self::GAME_INDEX_BITS
        );
        let mut bytes = [0; Self::SIZE];
        let mut buf = &mut bytes[..];
        hash.pack(&mut buf);
        buf.put_slice(&m.to_bytes());
        buf.put_u8(header.to_byte());
        month.pack(&mut buf);
        buf.put_uint(game.get() << 2 | u64::from(outcome_bits(outcome)), 5);
        rating.pack_fixed(&mut buf);
        debug_assert!(buf.is_empty());
        IntermediateEntry(bytes)
    }

    #[inline]
    pub fn hash(&self) -> PositionKey {
        PositionKey::unpack(&mut &self.0[..PositionKey::SIZE])
    }

    #[inline]
    pub fn hash_prefix(&self) -> u64 {
        u64::from_be_bytes(self.0[..8].try_into().expect("8 bytes"))
    }

    #[inline]
    pub fn sort_key(&self) -> (u128, u64) {
        // key() + cell_key() + game_and_outcome()
        (
            u128::from_be_bytes(self.0[..16].try_into().expect("16 bytes")),
            u64::from_be_bytes(self.0[14..22].try_into().expect("8 bytes")),
        )
    }

    #[inline]
    pub fn key(&self) -> &[u8; Self::KEY_SIZE] {
        self.0[..Self::KEY_SIZE].try_into().expect("key")
    }

    #[inline]
    pub fn month(&self) -> Month {
        Month::unpack(&mut &self.0[15..17])
    }

    #[inline]
    pub fn cell_key(&self) -> CellKey {
        CellKey::from_packed([self.0[12], self.0[13]], self.0[14], self.month())
    }

    #[inline]
    pub fn uci_move(&self) -> PackedUciMove {
        self.cell_key().uci_move()
    }

    #[inline]
    pub fn group_header(&self) -> GroupHeader {
        self.cell_key().group_header()
    }

    #[inline]
    fn game_and_outcome(&self) -> u64 {
        (&self.0[17..22]).get_uint(5)
    }

    #[inline]
    pub fn game_index(&self) -> GameIndex {
        GameIndex::new(self.game_and_outcome() >> 2).expect("38 bits")
    }

    #[inline]
    pub fn outcome(&self) -> KnownOutcome {
        outcome_from_bits((self.game_and_outcome() & 0b11) as u8).expect("valid outcome")
    }

    #[inline]
    pub fn rating(&self) -> Rating {
        Rating::unpack_fixed(&mut &self.0[22..24])
    }
}

impl fmt::Debug for IntermediateEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntermediateEntry")
            .field("hash", &self.hash())
            .field("uci_move", &format_args!("{}", self.uci_move().unpack()))
            .field("group_header", &self.group_header())
            .field("month", &self.month())
            .field("game_index", &self.game_index())
            .field("outcome", &self.outcome())
            .field("rating", &self.rating())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_intermediate_entry_roundtrip() {
        arbtest(|u| {
            let hash: PositionKey = u.arbitrary()?;
            let month: Month = u.arbitrary()?;
            let m = PackedUciMove::pack(u.arbitrary()?);
            let header: GroupHeader = u.arbitrary()?;
            let rating: Rating = u.arbitrary()?;
            let outcome: KnownOutcome = u.arbitrary()?;
            let game =
                GameIndex::new(u.int_in_range(0..=(1 << IntermediateEntry::GAME_INDEX_BITS) - 1)?)
                    .unwrap();

            let entry = IntermediateEntry::new(hash, month, m, header, rating, outcome, game);

            assert_eq!(entry.hash(), hash);
            assert_eq!(entry.month(), month);
            assert_eq!(entry.uci_move(), m);
            assert_eq!(entry.group_header(), header);
            assert_eq!(entry.game_index(), game);
            assert_eq!(entry.outcome(), outcome);
            assert_eq!(entry.rating(), rating);
            Ok(())
        });
    }
}
