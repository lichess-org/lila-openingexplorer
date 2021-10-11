use crate::{
    api::ColorProxy,
    model::{Speed, UserName},
};
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr, FromInto};
use shakmaty::Color;
use std::io::{self, Read, Write};

#[serde_as]
#[derive(Serialize)]
pub struct GameInfo {
    #[serde_as(as = "Option<FromInto<ColorProxy>>")]
    winner: Option<Color>,
    speed: Speed,
    rated: bool,
    white: Player,
    black: Player,
    year: u16,
}

impl GameInfo {
    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(match self.winner {
            Some(Color::Black) => 0,
            Some(Color::White) => 1,
            None => 2,
        })?;
        writer.write_u8(match self.speed {
            Speed::UltraBullet => 0,
            Speed::Bullet => 1,
            Speed::Blitz => 2,
            Speed::Rapid => 3,
            Speed::Classical => 4,
            Speed::Correspondence => 5,
        })?;
        writer.write_u8(if self.rated { 1 } else { 0 })?;
        self.white.write(writer)?;
        self.black.write(writer)?;
        writer.write_u16::<LittleEndian>(self.year)
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<GameInfo> {
        let winner = match reader.read_u8()? {
            0 => Some(Color::Black),
            1 => Some(Color::White),
            2 => None,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let speed = match reader.read_u8()? {
            0 => Speed::UltraBullet,
            1 => Speed::Bullet,
            2 => Speed::Blitz,
            3 => Speed::Rapid,
            4 => Speed::Classical,
            5 => Speed::Correspondence,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let rated = match reader.read_u8()? {
            0 => false,
            1 => true,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let white = Player::read(reader)?;
        let black = Player::read(reader)?;
        let year = reader.read_u16::<LittleEndian>()?;
        Ok(GameInfo {
            winner,
            speed,
            rated,
            white,
            black,
            year,
        })
    }
}

#[serde_as]
#[derive(Serialize)]
struct Player {
    #[serde_as(as = "DisplayFromStr")]
    name: UserName,
    rating: u16,
}

impl Player {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.name.as_bytes().len() as u8)?;
        writer.write_all(self.name.as_bytes())?;
        writer.write_u16::<LittleEndian>(self.rating)
    }

    fn read<R: Read>(reader: &mut R) -> io::Result<Player> {
        let len = reader.read_u8()?;
        let mut buf = vec![0; len as usize];
        reader.read_exact(&mut buf)?;
        let name = UserName::from_bytes(&buf)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let rating = reader.read_u16::<LittleEndian>()?;
        Ok(Player { name, rating })
    }
}
