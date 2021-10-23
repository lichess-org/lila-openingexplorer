use byteorder::{BigEndian, ByteOrder as _, LittleEndian};
use sha1::{Digest, Sha1};
use shakmaty::{variant::Variant, Color};

use crate::model::{Month, UserId};

#[derive(Debug)]
pub struct KeyBuilder {
    base: u128,
}

impl KeyBuilder {
    pub fn personal(user: &UserId, color: Color) -> KeyBuilder {
        let mut hash = Sha1::new();
        hash.update(&[color.char() as u8]);
        hash.update(user.as_str());
        let buf = hash.finalize();
        KeyBuilder {
            base: LittleEndian::read_u128(buf.as_slice()),
        }
    }

    pub fn master() -> KeyBuilder {
        KeyBuilder { base: 0 }
    }

    pub fn with_zobrist(&self, variant: Variant, zobrist: u128) -> KeyPrefix {
        // Zobrist hashes are the opposite of cryptographically secure. An
        // attacker could efficiently construct a position such that a record
        // will appear in the opening explorer of another player. This is not
        // completely trivial, and theres very little incentive, so we will
        // switch to a more expensive hash function only once required,
        // and then also stop using SHA1 in with_user_pov().
        KeyPrefix {
            prefix: (self.base
                ^ zobrist
                ^ (match variant {
                    Variant::Chess => 0,
                    Variant::Antichess => 0x44782fce075483666c81899cb65921c9,
                    Variant::Atomic => 0x66ccbd680f655d562689ca333c5e2a42,
                    Variant::Crazyhouse => 0x9d04db38ca4d923d82ff24eb9530e986,
                    Variant::Horde => 0xc29dfb1076aa15186effd0d34cc60737,
                    Variant::KingOfTheHill => 0xdfb25d5df41fc5961e61f6b4ba613fbe,
                    Variant::RacingKings => 0x8e72f94307f96710b3910cf7e5808e0d,
                    Variant::ThreeCheck => 0xd19242bae967b40e7856bd1c71aa4220,
                }))
            .to_le_bytes(),
        }
    }
}

#[derive(Debug)]
pub struct KeyPrefix {
    prefix: [u8; 16],
}

impl KeyPrefix {
    pub const SIZE: usize = 12;

    pub fn with_month(&self, month: Month) -> Key {
        let mut buf = [0; Key::SIZE];
        buf[..KeyPrefix::SIZE].clone_from_slice(&self.prefix[..KeyPrefix::SIZE]);
        BigEndian::write_u16(&mut buf[KeyPrefix::SIZE..], u16::from(month));
        Key(buf)
    }
}

#[derive(Debug)]
pub struct Key([u8; Key::SIZE]);

impl Key {
    pub const SIZE: usize = KeyPrefix::SIZE + 2;

    pub fn into_bytes(self) -> [u8; Self::SIZE] {
        self.0
    }
}
