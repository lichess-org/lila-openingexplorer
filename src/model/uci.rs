use std::convert::TryFrom;

use bytes::{Buf, BufMut};
use shakmaty::{uci::Uci, Role, Square};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct RawUci(u16);

impl RawUci {
    pub fn read<B: Buf>(buf: &mut B) -> RawUci {
        RawUci(buf.get_u16_le())
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        buf.put_u16_le(self.0);
    }
}

impl From<RawUci> for Uci {
    fn from(raw: RawUci) -> Uci {
        let from = Square::new(u32::from(raw.0 & 63));
        let to = Square::new(u32::from((raw.0 >> 6) & 63));
        let role = Role::try_from(raw.0 >> 12).ok();
        if from == to {
            match role {
                Some(role) => Uci::Put { role, to },
                None => Uci::Null,
            }
        } else {
            Uci::Normal {
                from,
                to,
                promotion: role,
            }
        }
    }
}

impl From<Uci> for RawUci {
    fn from(uci: Uci) -> RawUci {
        let (from, to, role) = match uci {
            Uci::Normal {
                from,
                to,
                promotion,
            } => (from, to, promotion),
            Uci::Put { role, to } => (to, to, Some(role)),
            Uci::Null => (Square::A1, Square::A1, None),
        };
        RawUci(
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
    fn test_uci_roundtrip() {
        let moves = [
            Uci::Null,
            Uci::Normal {
                from: Square::A1,
                to: Square::H8,
                promotion: None,
            },
            Uci::Normal {
                from: Square::A2,
                to: Square::A1,
                promotion: Some(Role::King),
            },
            Uci::Put {
                to: Square::A1,
                role: Role::Knight,
            },
        ];

        let mut buf = Vec::new();
        for uci in &moves {
            RawUci::from(uci.clone()).write(&mut buf);
        }

        let mut reader = &buf[..];
        for uci in moves {
            assert_eq!(uci, Uci::from(RawUci::read(&mut reader)));
        }
    }
}
