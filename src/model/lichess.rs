use std::{
    cmp::max,
    io::{self, Read, Write},
};

use byteorder::{ReadBytesExt as _, WriteBytesExt as _};

use crate::model::{write_uint, Speed};

#[derive(Copy, Clone)]
enum RatingGroup {
    Group1600,
    Group1800,
    Group2000,
    Group2200,
    Group2500,
    Group2800,
    Group3200,
}

enum Header {
    Group {
        rating_group: RatingGroup,
        speed: Speed,
        num_games: usize,
    },
    End,
}

impl Header {
    fn read<R: Read>(reader: &mut R) -> io::Result<Header> {
        let n = reader.read_u8()?;
        let speed = match n & 7 {
            0 => return Ok(Header::End),
            1 => Speed::UltraBullet,
            2 => Speed::Bullet,
            3 => Speed::Blitz,
            4 => Speed::Rapid,
            5 => Speed::Classical,
            6 => Speed::Correspondence,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let rating_group = match (n >> 3) & 7 {
            0 => RatingGroup::Group1600,
            1 => RatingGroup::Group1800,
            2 => RatingGroup::Group2000,
            3 => RatingGroup::Group2200,
            4 => RatingGroup::Group2500,
            5 => RatingGroup::Group2800,
            6 => RatingGroup::Group3200,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let at_least_num_games = usize::from(n >> 6);
        Ok(Header::Group {
            speed,
            rating_group,
            num_games: if at_least_num_games >= 3 {
                usize::from(reader.read_u8()?)
            } else {
                at_least_num_games
            },
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match *self {
            Header::End => writer.write_u8(0),
            Header::Group {
                speed,
                rating_group,
                num_games,
            } => {
                writer.write_u8(
                    (match speed {
                        Speed::UltraBullet => 1,
                        Speed::Bullet => 2,
                        Speed::Blitz => 3,
                        Speed::Rapid => 4,
                        Speed::Classical => 5,
                        Speed::Correspondence => 6,
                    }) | (match rating_group {
                        RatingGroup::Group1600 => 0,
                        RatingGroup::Group1800 => 1,
                        RatingGroup::Group2000 => 2,
                        RatingGroup::Group2200 => 3,
                        RatingGroup::Group2500 => 4,
                        RatingGroup::Group2800 => 5,
                        RatingGroup::Group3200 => 6,
                    } << 3)
                        | ((max(3, num_games) as u8) << 6),
                )?;
                if num_games >= 3 {
                    write_uint(writer, num_games as u64)?;
                }
                Ok(())
            }
        }
    }
}
