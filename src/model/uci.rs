use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use shakmaty::{uci::Uci, Role, Square};
use std::{
    convert::TryFrom,
    io::{self, Read, Write},
};

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

pub fn write_uci<W: Write>(writer: &mut W, uci: &Uci) -> io::Result<()> {
    let (from, to, role) = match *uci {
        Uci::Normal {
            from,
            to,
            promotion,
        } => (from, to, promotion),
        Uci::Put { role, to } => (to, to, Some(role)),
        Uci::Null => (Square::A1, Square::A1, None),
    };
    writer.write_u16::<LittleEndian>(
        u16::from(from) | (u16::from(to) << 6) | (role.map(u16::from).unwrap_or_default() << 12),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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

        let mut writer = Cursor::new(Vec::new());
        for uci in &moves {
            write_uci(&mut writer, uci).unwrap();
        }

        let mut reader = Cursor::new(writer.into_inner());
        for uci in moves {
            assert_eq!(uci, read_uci(&mut reader).unwrap());
        }
    }
}
