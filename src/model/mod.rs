mod game_id;

use byteorder::ReadBytesExt;
use std::io::{self, Read};

pub use game_id::{GameId, InvalidGameId};

fn read_uint<R: Read>(reader: &mut R) -> io::Result<u64> {
    let mut n = 0;
    let mut shift = 0;
    loop {
        let byte = reader.read_u8()?;
        n |= u64::from(byte & 127)
            .checked_shl(shift)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        if byte & 128 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(n)
}
