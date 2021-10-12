use crate::{
    api::ColorProxy,
    model::{read_uint, write_uint, Speed},
};
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use serde::Serialize;
use serde_with::{serde_as, FromInto};
use shakmaty::{ByColor, Color};
use std::{
    convert::TryFrom,
    io::{self, Read, Write},
};

#[serde_as]
#[derive(Serialize, Debug)]
pub struct GameInfo {
    #[serde_as(as = "Option<FromInto<ColorProxy>>")]
    pub winner: Option<Color>,
    pub speed: Speed,
    pub rated: bool,
    pub white: GameInfoPlayer,
    pub black: GameInfoPlayer,
    pub year: u32,
    #[serde(skip)]
    pub indexed: ByColor<bool>,
}

impl GameInfo {
    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(
            match self.speed {
                Speed::UltraBullet => 0,
                Speed::Bullet => 1,
                Speed::Blitz => 2,
                Speed::Rapid => 3,
                Speed::Classical => 4,
                Speed::Correspondence => 5,
            } | (match self.winner {
                Some(Color::Black) => 0,
                Some(Color::White) => 1,
                None => 2,
            } << 3)
                | (if self.rated { 1 } else { 0 } << 5)
                | (if self.indexed.white { 1 } else { 0 } << 6)
                | (if self.indexed.black { 1 } else { 0 } << 7),
        )?;
        self.white.write(writer)?;
        self.black.write(writer)?;
        write_uint(writer, u64::from(self.year))
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
        let winner = match (byte >> 3) & 3 {
            0 => Some(Color::Black),
            1 => Some(Color::White),
            2 => None,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let rated = (byte >> 5) & 1 == 1;
        let indexed = ByColor {
            white: (byte >> 6) & 1 == 1,
            black: (byte >> 7) & 1 == 1,
        };
        let white = GameInfoPlayer::read(reader)?;
        let black = GameInfoPlayer::read(reader)?;
        let year = u32::try_from(read_uint(reader)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(GameInfo {
            winner,
            speed,
            rated,
            white,
            black,
            year,
            indexed,
        })
    }
}

#[serde_as]
#[derive(Serialize, Debug)]
pub struct GameInfoPlayer {
    pub name: Option<String>,
    pub rating: Option<u16>,
}

impl GameInfoPlayer {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.name.as_ref().map_or(0, |s| s.len()) as u64)?;
        if let Some(name) = &self.name {
            writer.write_all(name.as_bytes())?;
        }
        writer.write_u16::<LittleEndian>(self.rating.unwrap_or(0))
    }

    fn read<R: Read>(reader: &mut R) -> io::Result<GameInfoPlayer> {
        let len = usize::try_from(read_uint(reader)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let mut buf = vec![0; len as usize];
        reader.read_exact(&mut buf)?;
        Ok(GameInfoPlayer {
            name: Some(
                String::from_utf8(buf)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            )
            .filter(|s| !s.is_empty()),
            rating: Some(reader.read_u16::<LittleEndian>()?).filter(|r| *r != 0),
        })
    }
}
