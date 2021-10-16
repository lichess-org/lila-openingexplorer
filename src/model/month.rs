use chrono::{DateTime, Datelike as _, Utc};
use std::{cmp::min, error::Error as StdError, fmt, str::FromStr};

const MAX_YEAR: u16 = 3000; // MAX_YEAR * 12 + 12 < 2^16

#[derive(Debug, Copy, Clone, Default)]
pub struct Month(pub u16);

impl Month {
    pub fn from_time_saturating(time: DateTime<Utc>) -> Month {
        let year = match time.year_ce() {
            (true, ce) => min(u32::from(MAX_YEAR), ce) as u16,
            (false, _) => 0,
        };

        Month(year * 12 + time.month0() as u16)
    }
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}/{:02}", self.0 / 12, self.0 % 12 + 1)
    }
}

impl FromStr for Month {
    type Err = InvalidMonth;

    fn from_str(s: &str) -> Result<Month, InvalidMonth> {
        let mut parts = s.splitn(2, '/');

        let year: u16 = parts
            .next()
            .expect("splitn non-empty")
            .parse()
            .map_err(|_| InvalidMonth)?;

        let month_plus_one: u16 = match parts.next() {
            Some(part) => part.parse().map_err(|_| InvalidMonth)?,
            None => 1,
        };

        if year <= MAX_YEAR && 1 <= month_plus_one && month_plus_one <= 12 {
            Ok(Month(year * 12 + month_plus_one - 1))
        } else {
            Err(InvalidMonth)
        }
    }
}

#[derive(Debug)]
pub struct InvalidMonth;

impl fmt::Display for InvalidMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid month")
    }
}

impl StdError for InvalidMonth {}
