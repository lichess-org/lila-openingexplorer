use std::ops::{AddAssign, Sub};

use bytes::{Buf, BufMut};
use serde::Serialize;
use shakmaty::{Color, Outcome};

use crate::model::{read_uint, write_uint};

#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize)]
pub struct Stats {
    #[serde(skip)]
    rating_sum: u64,
    white: u64,
    draws: u64,
    black: u64,
}

impl Stats {
    pub fn new_single(outcome: Outcome, rating: u16) -> Stats {
        Stats {
            rating_sum: u64::from(rating),
            white: u64::from(outcome.winner() == Some(Color::White)),
            black: u64::from(outcome.winner() == Some(Color::Black)),
            draws: u64::from(outcome.winner().is_none()),
        }
    }
}

impl AddAssign<&Stats> for Stats {
    fn add_assign(&mut self, rhs: &Stats) {
        self.rating_sum += rhs.rating_sum;
        self.white += rhs.white;
        self.draws += rhs.draws;
        self.black += rhs.black;
    }
}

impl<'a> Sub<&'a Stats> for &Stats {
    type Output = Stats;

    fn sub(self, other: &'a Stats) -> Stats {
        Stats {
            rating_sum: self.rating_sum - other.rating_sum,
            white: self.white - other.white,
            black: self.black - other.black,
            draws: self.draws - other.draws,
        }
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

    pub fn white(&self) -> u64 {
        self.white
    }

    pub fn black(&self) -> u64 {
        self.black
    }

    pub fn draws(&self) -> u64 {
        self.draws
    }

    fn average_rating_f64(&self) -> Option<f64> {
        if self.total() > 0 {
            Some(self.rating_sum as f64 / self.total() as f64)
        } else {
            None
        }
    }

    pub fn average_rating(&self) -> Option<u16> {
        self.average_rating_f64().map(|avg| avg.round() as u16)
    }

    pub fn performance(&self, color: Color) -> Option<i32> {
        // https://handbook.fide.com/chapter/B022017
        const DELTAS: [f64; 101] = [
            -800.0, -677.0, -589.0, -538.0, -501.0, -470.0, -444.0, -422.0, -401.0, -383.0, -366.0,
            -351.0, -336.0, -322.0, -309.0, -296.0, -284.0, -273.0, -262.0, -251.0, -240.0, -230.0,
            -220.0, -211.0, -202.0, -193.0, -184.0, -175.0, -166.0, -158.0, -149.0, -141.0, -133.0,
            -125.0, -117.0, -110.0, -102.0, -95.0, -87.0, -80.0, -72.0, -65.0, -57.0, -50.0, -43.0,
            -36.0, -29.0, -21.0, -14.0, -7.0, 0.0, 7.0, 14.0, 21.0, 29.0, 36.0, 43.0, 50.0, 57.0,
            65.0, 72.0, 80.0, 87.0, 95.0, 102.0, 110.0, 117.0, 125.0, 133.0, 141.0, 149.0, 158.0,
            166.0, 175.0, 184.0, 193.0, 202.0, 211.0, 220.0, 230.0, 240.0, 251.0, 262.0, 273.0,
            284.0, 296.0, 309.0, 322.0, 336.0, 351.0, 366.0, 383.0, 401.0, 422.0, 444.0, 470.0,
            501.0, 538.0, 589.0, 677.0, 800.0,
        ];

        self.average_rating_f64().map(|avg_opponent_rating| {
            let score = 100 * color.fold_wb(self.white, self.black) + 50 * self.draws;
            let p = (score as f64) / (self.total() as f64);
            let idx = p.trunc() as usize;
            (avg_opponent_rating
                + DELTAS[idx] * (1.0 - p.fract())
                + *DELTAS.get(idx + 1).unwrap_or(&800.0) * p.fract())
            .round() as i32
        })
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
    use quickcheck::{Arbitrary, Gen, quickcheck};

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

    #[test]
    fn test_performance() {
        let single = Stats {
            white: 1,
            draws: 0,
            black: 0,
            rating_sum: 1500,
        };
        assert_eq!(single.performance(Color::White), Some(2300));
        assert_eq!(single.performance(Color::Black), Some(700));

        let symmetrical = Stats {
            white: 123,
            draws: 10,
            black: 123,
            rating_sum: (123 + 10 + 123) * 987,
        };
        assert_eq!(symmetrical.performance(Color::White), Some(987));
        assert_eq!(symmetrical.performance(Color::Black), Some(987));

        let p5 = Stats {
            white: 5,
            draws: 0,
            black: 95,
            rating_sum: 0,
        };
        assert_eq!(p5.performance(Color::White), Some(-470));
        assert_eq!(p5.performance(Color::Black), Some(470));
    }
}
