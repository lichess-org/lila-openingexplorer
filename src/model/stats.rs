use std::ops::AddAssign;

use bytes::{Buf, BufMut};
use serde::Serialize;
use shakmaty::{Color, Outcome};

use crate::model::{read_uint, write_uint};

#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize)]
pub struct Stats {
    #[serde(skip)]
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
        self.white == 0 && self.draws == 0 && self.black == 0
    }

    pub fn is_single(&self) -> bool {
        self.total() == 1
    }

    pub fn average_rating(&self) -> Option<u64> {
        self.rating_sum.checked_div(self.total())
    }

    pub fn read<B: Buf>(buf: &mut B) -> Stats {
        let rating_sum = read_uint(buf);
        match read_uint(buf) {
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
                draws: read_uint(buf),
                black: read_uint(buf),
            },
        }
    }

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        write_uint(buf, self.rating_sum);
        match *self {
            Stats {
                white: 1,
                draws: 0,
                black: 0,
                ..
            } => write_uint(buf, 0),
            Stats {
                white: 0,
                draws: 0,
                black: 1,
                ..
            } => write_uint(buf, 1),
            Stats {
                white: 0,
                draws: 1,
                black: 0,
                ..
            } => write_uint(buf, 2),
            Stats {
                white,
                draws,
                black,
                ..
            } => {
                write_uint(buf, white + 3);
                write_uint(buf, draws);
                write_uint(buf, black);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::{quickcheck, Arbitrary, Gen};

    use super::*;

    impl Arbitrary for Stats {
        fn arbitrary(g: &mut Gen) -> Self {
            Stats {
                rating_sum: u64::from(u32::arbitrary(g)),
                white: u64::from(u32::arbitrary(g)),
                draws: u64::from(u32::arbitrary(g)),
                black: u64::from(u32::arbitrary(g)),
            }
        }
    }

    quickcheck! {
        fn test_stats_roundtrip(stats: Stats) -> bool {
            let mut buf = Vec::new();
            stats.write(&mut buf);

            let mut reader = &buf[..];
            Stats::read(&mut reader) == stats
        }
    }
}
