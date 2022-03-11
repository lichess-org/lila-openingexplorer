use std::{
    convert::TryFrom,
    io::{self, Read},
};

use byteorder::{LittleEndian, ReadBytesExt as _};
use bytes::BufMut;
use shakmaty::{uci::Uci, Role, Square};

pub fn read_uci<R: Read>(reader: &mut R) -> io::Result<Uci> {
    let n = reader.read_u16::<LittleEndian>()?;
    let from = Square::new(u32::from(n & 63));
    let to = Square::new(u32::from((n >> 6) & 63));
    let role = Role::try_from(n >> 12).ok();
    Ok(if from == to {
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
    })
}

pub fn write_uci<B: BufMut>(buf: &mut B, uci: &Uci) {
    let (from, to, role) = match *uci {
        Uci::Normal {
            from,
            to,
            promotion,
        } => (from, to, promotion),
        Uci::Put { role, to } => (to, to, Some(role)),
        Uci::Null => (Square::A1, Square::A1, None),
    };
    buf.put_u16_le(
        u16::from(from) | (u16::from(to) << 6) | (role.map(u16::from).unwrap_or_default() << 12),
    );
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

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
            write_uci(&mut buf, uci);
        }

        let mut reader = Cursor::new(buf);
        for uci in moves {
            assert_eq!(uci, read_uci(&mut reader).unwrap());
        }
    }
}
