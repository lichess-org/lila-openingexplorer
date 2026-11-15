use std::hash::{Hash, Hasher};

use arbitrary::Arbitrary;
use bytes::{Buf, BufMut};
use shakmaty::{
    EnPassantMode, Position,
    variant::{Variant, VariantPosition},
    zobrist::Zobrist128,
};

/// Key of a position in the positions and cold positions column families:
/// a Zobrist hash of the position with the variant mixed in, truncated to
/// 96 bits. Unlike a plain Zobrist hash, keys of the same board in different
/// variants differ.
#[derive(Arbitrary, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PositionKey {
    inner: [u8; PositionKey::SIZE],
}

impl Hash for PositionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.prefix());
    }
}

impl nohash_hasher::IsEnabled for PositionKey {}

impl PositionKey {
    pub const SIZE: usize = 12;

    pub fn new(pos: &VariantPosition) -> PositionKey {
        PositionKey::from_zobrist(pos.zobrist_hash(EnPassantMode::Legal), pos.variant())
    }

    /// The key of a position of the variant, given its
    /// `zobrist_hash(EnPassantMode::Legal)` (for example computed
    /// incrementally).
    pub fn from_zobrist(hash: Zobrist128, variant: Variant) -> PositionKey {
        let variant_hash = Zobrist128(match variant {
            Variant::Chess => 0,
            Variant::Atomic => 0x448f_8a94_c263_0790_6c4d_732b_52bf_d09c,
            Variant::Antichess => 0xc37c_461f_0c94_0a7a_ed6e_5574_5b53_36a8,
            Variant::KingOfTheHill => 0xcfc7_524f_c49e_0cc8_3424_ce23_54a3_0465,
            Variant::ThreeCheck => 0x125f_3ede_51b1_6a4f_1a83_259a_78ae_c28b,
            Variant::Crazyhouse => 0x123d_7313_ce55_0520_d96a_8f0b_7ff9_a3eb,
            Variant::RacingKings => 0x17d0_49d2_5cdb_93df_97e4_5dc6_26d4_f3ce,
            Variant::Horde => 0x2b0e_f487_527a_f875_8e8e_2bfa_05bd_d376,
        });

        let mut inner = [0; PositionKey::SIZE];
        inner.copy_from_slice(&u128::from(hash ^ variant_hash).to_be_bytes()[..PositionKey::SIZE]);
        PositionKey { inner }
    }

    pub fn from_bytes(inner: [u8; PositionKey::SIZE]) -> PositionKey {
        PositionKey { inner }
    }

    pub fn as_bytes(&self) -> &[u8; PositionKey::SIZE] {
        &self.inner
    }

    pub fn prefix(&self) -> u64 {
        u64::from_be_bytes(self.inner[..8].try_into().expect("8 bytes"))
    }

    pub fn pack<B: BufMut>(&self, buf: &mut B) {
        buf.put_slice(&self.inner);
    }

    pub fn unpack<B: Buf>(buf: &mut B) -> Self {
        let mut inner = [0; PositionKey::SIZE];
        buf.copy_to_slice(&mut inner);
        PositionKey { inner }
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::{Chess, uci::UciMove};

    use super::*;

    #[test]
    fn test_new() {
        let mut pos = Chess::default();
        let m = "e2e4".parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        let hash = pos.update_zobrist_hash(
            pos.zobrist_hash(EnPassantMode::Legal),
            m,
            EnPassantMode::Legal,
        );
        pos.play_unchecked(m);
        assert_eq!(
            PositionKey::new(&VariantPosition::Chess(pos.clone())),
            PositionKey::from_zobrist(hash.unwrap(), Variant::Chess)
        );
    }

    #[test]
    fn test_variants_differ() {
        let keys: Vec<PositionKey> = Variant::ALL
            .into_iter()
            .map(|variant| PositionKey::new(&VariantPosition::new(variant)))
            .collect();
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Even where the plain hashes agree.
        let hash = |variant| {
            VariantPosition::new(variant).zobrist_hash::<Zobrist128>(EnPassantMode::Legal)
        };
        assert_eq!(hash(Variant::Chess), hash(Variant::KingOfTheHill));
    }
}
