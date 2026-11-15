use std::{fmt, num::NonZeroU8, str::FromStr};

use arbitrary::Arbitrary;
use btoi::btou;
use bytes::{Buf, BufMut};
use thiserror::Error;

#[derive(Arbitrary, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month {
    since_jan_1900: u16,
}

impl Month {
    pub fn from_months_since_jan_1900(since_jan_1900: u16) -> Self {
        Self { since_jan_1900 }
    }

    pub fn months_since_jan_1900(self) -> u16 {
        self.since_jan_1900
    }

    pub fn pack<B: BufMut>(&self, buf: &mut B) {
        buf.put_u16(self.since_jan_1900);
    }

    pub fn unpack<B: Buf>(buf: &mut B) -> Self {
        Self {
            since_jan_1900: buf.get_u16(),
        }
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<Self, InvalidMonth> {
        let mut parts = bytes.splitn(2, |&b| b == b'-');
        let year = parts
            .next()
            .and_then(|part| btou::<u16>(part).ok())
            .ok_or(InvalidMonth)?;
        let month = parts
            .next()
            .and_then(|part| btou::<u16>(part).ok())
            .ok_or(InvalidMonth)?;
        Month::from_year_and_month(year, month)
    }

    /// The month (UTC) of a time in milliseconds since the epoch, saturating
    /// at the representable months.
    pub fn from_unix_ms(ms: u64) -> Month {
        // Civil from days (Howard Hinnant), for days since 1970-01-01.
        let days = ms / 86_400_000;
        let z = days + 719_468;
        let era = z / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + u64::from(month <= 2);
        Month::from_year_and_month(u16::try_from(year).unwrap_or(u16::MAX), month as u16).unwrap_or(
            Month {
                since_jan_1900: u16::MAX,
            },
        )
    }

    pub fn from_year_and_month(year: u16, month: u16) -> Result<Self, InvalidMonth> {
        if !(1..=12).contains(&month) {
            return Err(InvalidMonth);
        }
        Ok(Month {
            since_jan_1900: year
                .checked_sub(1900)
                .and_then(|year_since_1900| year_since_1900.checked_mul(12))
                .and_then(|months_since_1900| months_since_1900.checked_add(month - 1))
                .ok_or(InvalidMonth)?,
        })
    }
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let year = self.since_jan_1900 / 12 + 1900;
        let month = self.since_jan_1900 % 12 + 1;
        write!(f, "{}-{:02}", year, month)
    }
}

impl fmt::Debug for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Month({})", self)
    }
}

impl FromStr for Month {
    type Err = InvalidMonth;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_ascii(s.as_bytes())
    }
}

#[derive(Debug, Error)]
#[error("invalid month")]
pub struct InvalidMonth;

/// A year or a month, like `2020` or `2020-03` (also `2020/03`), for
/// inclusive ranges of months.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaxMonth {
    year_since_1900: u16,
    month: Option<NonZeroU8>,
}

impl LaxMonth {
    fn month_in_year_or(self, default: u8) -> Month {
        Month::from_months_since_jan_1900(
            self.year_since_1900 * 12 + u16::from(self.month.map_or(default, NonZeroU8::get) - 1),
        )
    }

    pub fn earliest_month(self) -> Month {
        self.month_in_year_or(1)
    }

    pub fn latest_month(self) -> Month {
        self.month_in_year_or(12)
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<LaxMonth, InvalidMonth> {
        let mut parts = bytes.splitn(2, |&b| b == b'-' || b == b'/');
        let year_since_1900 = parts
            .next()
            .and_then(|part| btou::<u16>(part).ok())
            .and_then(|year| year.checked_sub(1900))
            .ok_or(InvalidMonth)?;
        let month = parts
            .next()
            .map(|part| {
                btou::<u8>(part)
                    .ok()
                    .filter(|&m| m <= 12)
                    .and_then(NonZeroU8::new)
                    .ok_or(InvalidMonth)
            })
            .transpose()?;
        // The latest month (and therefore every month) must be representable.
        year_since_1900
            .checked_mul(12)
            .and_then(|months| months.checked_add(u16::from(month.map_or(12, NonZeroU8::get) - 1)))
            .ok_or(InvalidMonth)?;
        Ok(LaxMonth {
            year_since_1900,
            month,
        })
    }
}

impl FromStr for LaxMonth {
    type Err = InvalidMonth;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_ascii(s.as_bytes())
    }
}

#[cfg(test)]
mod test {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_from_unix_ms() {
        assert_eq!(Month::from_unix_ms(0).to_string(), "1970-01");
        // 2022-04-28 07:45:35 UTC, lastMoveAt of a lila export.
        assert_eq!(
            Month::from_unix_ms(1_651_131_935_030).to_string(),
            "2022-04"
        );
        // 2024-02-29 23:59:59.999 and 2024-03-01 00:00 UTC.
        assert_eq!(
            Month::from_unix_ms(1_709_251_199_999).to_string(),
            "2024-02"
        );
        assert_eq!(
            Month::from_unix_ms(1_709_251_200_000).to_string(),
            "2024-03"
        );
        // 2025-12-31 23:59:59.999 and 2026-01-01 00:00 UTC.
        assert_eq!(
            Month::from_unix_ms(1_767_225_599_999).to_string(),
            "2025-12"
        );
        assert_eq!(
            Month::from_unix_ms(1_767_225_600_000).to_string(),
            "2026-01"
        );
        // Saturating.
        let _ = Month::from_unix_ms(u64::MAX);
    }

    #[test]
    fn test_str() {
        assert_eq!(Month::from_months_since_jan_1900(0).to_string(), "1900-01");
    }

    #[test]
    fn test_str_roundtrip() {
        arbtest(|u| {
            let month: Month = u.arbitrary()?;

            assert_eq!(Month::from_str(&month.to_string()).unwrap(), month);
            Ok(())
        });
    }

    #[test]
    fn test_lax_month_str_roundtrip() {
        arbtest(|u| {
            let month: Month = u.arbitrary()?;

            let lax = LaxMonth::from_str(&month.to_string()).unwrap();
            assert_eq!(lax.earliest_month(), month);
            assert_eq!(lax.latest_month(), month);
            Ok(())
        });
    }

    #[test]
    fn test_lax_month() {
        let lax =
            |s: &str| LaxMonth::from_str(s).map(|lax| (lax.earliest_month(), lax.latest_month()));
        let month = |s: &str| Month::from_str(s).unwrap();
        assert_eq!(lax("2020").unwrap(), (month("2020-01"), month("2020-12")));
        assert_eq!(
            lax("2020/03").unwrap(),
            (month("2020-03"), month("2020-03"))
        );
        assert_eq!(lax("1900").unwrap(), (month("1900-01"), month("1900-12")));
        assert!(lax("1899").is_err());
        assert!(lax("2020-0").is_err());
        assert!(lax("2020-13").is_err());
        assert!(lax("2020-").is_err());
        assert!(lax("2020-01-01").is_err());
        // Months since 1900 must fit in u16.
        assert!(lax("7360").is_ok());
        assert!(lax("7361").is_err());
        assert!(lax("7361-04").is_ok());
        assert!(lax("7361-05").is_err());
    }

    #[test]
    fn test_pack_roundtrip() {
        arbtest(|u| {
            let month: Month = u.arbitrary()?;

            let mut buf = Vec::new();
            month.pack(&mut buf);
            let unpacked = Month::unpack(&mut &buf[..]);
            assert_eq!(month, unpacked);
            Ok(())
        });
    }

    #[test]
    fn test_packed_ord() {
        arbtest(|u| {
            let left: Month = u.arbitrary()?;
            let right: Month = u.arbitrary()?;

            let mut left_buf = Vec::new();
            left.pack(&mut left_buf);

            let mut right_buf = Vec::new();
            right.pack(&mut right_buf);

            assert_eq!(left_buf.cmp(&right_buf), left.cmp(&right));
            Ok(())
        });
    }
}
