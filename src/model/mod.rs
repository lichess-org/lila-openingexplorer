mod game_id;

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use shakmaty::uci::Uci;
use shakmaty::{Role, Square};
use std::convert::TryFrom;
use std::io::{self, Read, Write};

pub use game_id::{GameId, InvalidGameId};

fn read_uint<R: Read>(reader: &mut R) -> io::Result<u64> {
    let mut n = 0;
    for shift in (0..).step_by(7) {
        let byte = reader.read_u8()?;
        n |= u64::from(byte & 127)
            .checked_shl(shift)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        if byte & 128 == 0 {
            break;
        }
    }
    Ok(n)
}

fn write_uint<W: Write>(writer: &mut W, mut n: u64) -> io::Result<()> {
    while n > 127 {
        writer.write_u8((n as u8 & 127) | 128)?;
        n >>= 7;
    }
    writer.write_u8(n as u8)
}

fn read_uci<R: Read>(reader: &mut R) -> io::Result<Uci> {
    let n = reader.read_u16::<LittleEndian>()?;
    let from = Square::try_from(n & 63).unwrap();
    let to = Square::try_from((n >> 6) & 63).unwrap();
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

fn write_uci<W: Write>(writer: &mut W, uci: Uci) -> io::Result<()> {
    let (from, to, role) = match uci {
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
    use quickcheck::quickcheck;
    use std::io::Cursor;

    quickcheck! {
        fn uint_roundtrip(n: u64) -> bool {
            let mut writer = Cursor::new(Vec::new());
            write_uint(&mut writer, n).unwrap();

            let mut reader = Cursor::new(writer.into_inner());
            read_uint(&mut reader).unwrap() == n
        }
    }
}
