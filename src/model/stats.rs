use std::{
    io::{self, Read, Write},
    ops::AddAssign,
};

use serde::Serialize;
use shakmaty::{Color, Outcome};

use crate::model::{read_uint, write_uint};

#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize)]
pub struct Stats {
    pub rating_sum: u64,
    pub white: u64,
    pub draws: u64,
    pub black: u64,
}

impl Stats {
    pub fn new_single(outcome: Outcome, rating: u16) -> Stats {
        Stats {
            rating_sum: u64::from(rating),
            white: if outcome.winner() == Some(Color::White) {
                1
            } else {
                0
            },
            black: if outcome.winner() == Some(Color::Black) {
                1
            } else {
                0
            },
            draws: if outcome.winner().is_none() { 1 } else { 0 },
        }
    }
}

impl AddAssign for Stats {
    fn add_assign(&mut self, rhs: Stats) {
        self.rating_sum += rhs.rating_sum;
        self.white += rhs.white;
        self.draws += rhs.draws;
        self.black += rhs.black;
    }
}

impl Stats {
    pub fn total(&self) -> u64 {
        self.white + self.draws + self.black
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    pub fn is_single(&self) -> bool {
        self.total() == 1
    }

    pub fn average_rating(&self) -> Option<u64> {
        self.rating_sum.checked_div(self.total())
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<Stats> {
        let rating_sum = read_uint(reader)?;
        Ok(match read_uint(reader)? {
            0 => Stats {
                rating_sum,
                white: 1,
                draws: 0,
                black: 0,
            },
            1 => Stats {
                rating_sum,
                white: 0,
                draws: 0,
                black: 1,
            },
            2 => Stats {
                rating_sum,
                white: 0,
                draws: 1,
                black: 0,
            },
            white_plus_three => Stats {
                rating_sum,
                white: white_plus_three - 3,
                draws: read_uint(reader)?,
                black: read_uint(reader)?,
            },
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.rating_sum)?;
        match *self {
            Stats {
                white: 1,
                draws: 0,
                black: 0,
                ..
            } => write_uint(writer, 0),
            Stats {
                white: 0,
                draws: 0,
                black: 1,
                ..
            } => write_uint(writer, 1),
            Stats {
                white: 0,
                draws: 1,
                black: 0,
                ..
            } => write_uint(writer, 2),
            Stats {
                white,
                draws,
                black,
                ..
            } => {
                write_uint(writer, white + 3)?;
                write_uint(writer, draws)?;
                write_uint(writer, black)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use quickcheck::{quickcheck, Arbitrary, Gen};

    use super::*;

    impl Arbitrary for Stats {
        fn arbitrary(g: &mut Gen) -> Self {
            Stats {
                white: u64::from(u32::arbitrary(g)),
                draws: u64::from(u32::arbitrary(g)),
                black: u64::from(u32::arbitrary(g)),
            }
        }
    }

    quickcheck! {
        fn test_stats_roundtrip(stats: Stats) -> bool {
            let mut cursor = Cursor::new(Vec::new());
            stats.write(&mut cursor).unwrap();

            let mut cursor = Cursor::new(cursor.into_inner());
            Stats::read(&mut cursor).unwrap() == stats
        }
    }
}
