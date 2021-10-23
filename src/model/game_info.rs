use std::{
    convert::{TryFrom, TryInto},
    io::{self, Read, Write},
};

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use serde::{Deserialize, Serialize};
use shakmaty::{ByColor, Color, Outcome};

use crate::model::{read_uint, write_uint, Mode, Month, Speed};

#[derive(Debug)]
pub struct GameInfo {
    pub outcome: Outcome,
    pub speed: Speed,
    pub mode: Mode,
    pub players: ByColor<GameInfoPlayer>,
    pub month: Month,
    pub indexed: ByColor<bool>,
}

impl GameInfo {
    pub const SIZE_HINT: usize = 1 + 2 * (1 + 20 + 2) + 2;

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(
            match self.speed {
                Speed::UltraBullet => 0,
                Speed::Bullet => 1,
                Speed::Blitz => 2,
                Speed::Rapid => 3,
                Speed::Classical => 4,
                Speed::Correspondence => 5,
            } | (match self.outcome {
                Outcome::Decisive {
                    winner: Color::Black,
                } => 0,
                Outcome::Decisive {
                    winner: Color::White,
                } => 1,
                Outcome::Draw => 2,
            } << 3)
                | (if self.mode.is_rated() { 1 } else { 0 } << 5)
                | (if self.indexed.white { 1 } else { 0 } << 6)
                | (if self.indexed.black { 1 } else { 0 } << 7),
        )?;
        self.players.white.write(writer)?;
        self.players.black.write(writer)?;
        writer.write_u16::<LittleEndian>(u16::from(self.month))
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<GameInfo> {
        let byte = reader.read_u8()?;
        let speed = match byte & 7 {
            0 => Speed::UltraBullet,
            1 => Speed::Bullet,
            2 => Speed::Blitz,
            3 => Speed::Rapid,
            4 => Speed::Classical,
            5 => Speed::Correspondence,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let outcome = match (byte >> 3) & 3 {
            0 => Outcome::Decisive {
                winner: Color::Black,
            },
            1 => Outcome::Decisive {
                winner: Color::White,
            },
            2 => Outcome::Draw,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let mode = Mode::from_rated((byte >> 5) & 1 == 1);
        let indexed = ByColor {
            white: (byte >> 6) & 1 == 1,
            black: (byte >> 7) & 1 == 1,
        };
        let players = ByColor {
            white: GameInfoPlayer::read(reader)?,
            black: GameInfoPlayer::read(reader)?,
        };
        let month = reader
            .read_u16::<LittleEndian>()?
            .try_into()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(GameInfo {
            outcome,
            speed,
            mode,
            players,
            month,
            indexed,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GameInfoPlayer {
    pub name: String,
    pub rating: u16,
}

impl GameInfoPlayer {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.name.len() as u64)?;
        writer.write_all(self.name.as_bytes())?;
        writer.write_u16::<LittleEndian>(self.rating)
    }

    fn read<R: Read>(reader: &mut R) -> io::Result<GameInfoPlayer> {
        let len = usize::try_from(read_uint(reader)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let mut buf = vec![0; len as usize];
        reader.read_exact(&mut buf)?;
        Ok(GameInfoPlayer {
            name: String::from_utf8(buf)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            rating: reader.read_u16::<LittleEndian>()?,
        })
    }
}
