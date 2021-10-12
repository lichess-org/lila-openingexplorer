use crate::model::{read_uint, write_uint};
use serde::Serialize;
use shakmaty::{Color, Outcome};
use std::{
    io::{self, Read, Write},
    ops::AddAssign,
};

#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize)]
pub struct Stats {
    pub white: u64,
    pub draws: u64,
    pub black: u64,
}

impl From<Outcome> for Stats {
    fn from(outcome: Outcome) -> Stats {
        Stats {
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

    pub fn read<R: Read>(reader: &mut R) -> io::Result<Stats> {
        Ok(Stats {
            white: read_uint(reader)?,
            draws: read_uint(reader)?,
            black: read_uint(reader)?,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_uint(writer, self.white)?;
        write_uint(writer, self.draws)?;
        write_uint(writer, self.black)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{quickcheck, Arbitrary, Gen};
    use std::io::Cursor;

    impl Arbitrary for Stats {
        fn arbitrary(g: &mut Gen) -> Self {
            Stats {
                white: u64::arbitrary(g),
                draws: u64::arbitrary(g),
                black: u64::arbitrary(g),
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
