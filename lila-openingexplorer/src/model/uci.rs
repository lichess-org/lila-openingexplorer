use std::{convert::TryFrom, fmt};

use bytes::{Buf, BufMut};
use shakmaty::{uci::UciMove, Role, Square};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct RawUciMove(u16);

impl RawUciMove {
    pub fn read<B: Buf>(buf: &mut B) -> RawUciMove {
        RawUciMove(buf.get_u16_le())
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        buf.put_u16_le(self.0);
    }
}

impl fmt::Debug for RawUciMove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RawUciMove({})", UciMove::from(*self))
    }
}

impl nohash_hasher::IsEnabled for RawUciMove {}

impl From<RawUciMove> for UciMove {
    fn from(raw: RawUciMove) -> UciMove {
        let from = Square::new(u32::from(raw.0 & 63));
        let to = Square::new(u32::from((raw.0 >> 6) & 63));
        let role = Role::try_from(raw.0 >> 12).ok();
        if from == to {
            match role {
                Some(role) => UciMove::Put { role, to },
                None => UciMove::Null,
            }
        } else {
            UciMove::Normal {
                from,
                to,
                promotion: role,
            }
        }
    }
}

impl From<UciMove> for RawUciMove {
    fn from(uci: UciMove) -> RawUciMove {
        let (from, to, role) = match uci {
            UciMove::Normal {
                from,
                to,
                promotion,
            } => (from, to, promotion),
            UciMove::Put { role, to } => (to, to, Some(role)),
            UciMove::Null => (Square::A1, Square::A1, None),
        };
        RawUciMove(
            u16::from(from)
                | (u16::from(to) << 6)
                | (role.map(u16::from).unwrap_or_default() << 12),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uci_move_roundtrip() {
        let moves = [
            UciMove::Null,
            UciMove::Normal {
                from: Square::A1,
                to: Square::H8,
                promotion: None,
            },
            UciMove::Normal {
                from: Square::A2,
                to: Square::A1,
                promotion: Some(Role::King),
            },
            UciMove::Put {
                to: Square::A1,
                role: Role::Knight,
            },
        ];

        let mut buf = Vec::new();
        for uci in &moves {
            RawUciMove::from(uci.clone()).write(&mut buf);
        }

        let mut reader = &buf[..];
        for uci in moves {
            assert_eq!(uci, UciMove::from(RawUciMove::read(&mut reader)));
        }
    }
}
