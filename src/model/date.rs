use std::{cmp::min, convert::TryFrom, error::Error as StdError, fmt, str::FromStr};

use chrono::{DateTime, Datelike as _, Utc};

const MAX_YEAR: u16 = 3000; // MAX_YEAR * 12 + 12 < 2^16

#[derive(Debug, Copy, Clone, Default, Ord, PartialOrd, Eq, PartialEq)]
pub struct Year(u16);

impl Year {
    pub fn max_value() -> Year {
        Year(MAX_YEAR)
    }

    pub fn add_years_saturating(self, years: u16) -> Year {
        min(Year(self.0.saturating_add(years)), Year::max_value())
    }
}

impl From<Year> for u16 {
    fn from(Year(year): Year) -> u16 {
        year
    }
}

impl TryFrom<u16> for Year {
    type Error = InvalidYear;

    fn try_from(year: u16) -> Result<Year, InvalidYear> {
        if year <= Year::max_value().0 {
            Ok(Year(year))
        } else {
            Err(InvalidYear)
        }
    }
}

#[derive(Debug)]
pub struct InvalidYear;

impl fmt::Display for InvalidYear {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid year")
    }
}

impl StdError for InvalidYear {}

#[derive(Debug, Copy, Clone, Default, Ord, PartialOrd, Eq, PartialEq)]
pub struct Month(u16);

impl Month {
    pub fn max_value() -> Month {
        Month(MAX_YEAR * 12 + 11)
    }

    pub fn from_time_saturating(time: DateTime<Utc>) -> Month {
        let year = match time.year_ce() {
            (true, ce) => min(u32::from(MAX_YEAR), ce) as u16,
            (false, _) => 0,
        };

        Month(year * 12 + time.month0() as u16)
    }

    pub fn add_months_saturating(self, months: u16) -> Month {
        min(Month(self.0.saturating_add(months)), Month::max_value())
    }
}

impl From<Month> for u16 {
    fn from(Month(month): Month) -> u16 {
        month
    }
}

impl TryFrom<u16> for Month {
    type Error = InvalidMonth;

    fn try_from(month: u16) -> Result<Month, InvalidMonth> {
        if month <= Month::max_value().0 {
            Ok(Month(month))
        } else {
            Err(InvalidMonth)
        }
    }
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.0 / 12, self.0 % 12 + 1)
    }
}

impl FromStr for Month {
    type Err = InvalidMonth;

    fn from_str(s: &str) -> Result<Month, InvalidMonth> {
        match s.split_once(|ch| ch == '-' || ch == '/') {
            Some((year_part, month_part)) => {
                let year: u16 = year_part.parse().map_err(|_| InvalidMonth)?;
                let month_plus_one: u16 = month_part.parse().map_err(|_| InvalidMonth)?;

                if year <= MAX_YEAR && 1 <= month_plus_one && month_plus_one <= 12 {
                    Ok(Month(year * 12 + month_plus_one - 1))
                } else {
                    Err(InvalidMonth)
                }
            }
            None => Err(InvalidMonth),
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

#[cfg(test)]
mod tests {
    use quickcheck::{Arbitrary, Gen};

    use super::*;

    impl Arbitrary for Month {
        fn arbitrary(g: &mut Gen) -> Month {
            Month(u16::arbitrary(g) % (u16::from(Month::max_value()) + 1))
        }
    }
}
