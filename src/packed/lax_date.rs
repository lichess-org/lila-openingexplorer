use std::{fmt, num::NonZeroU8, str::FromStr};

use arbitrary::{Arbitrary, Unstructured};
use btoi::btou;
use bytes::{Buf, BufMut};
use thiserror::Error;

use crate::packed::Month;

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LaxDate {
    year_since_1900: u8,
    month: Option<NonZeroU8>,
    day: Option<NonZeroU8>,
}

impl LaxDate {
    pub fn year(&self) -> u32 {
        u32::from(self.year_since_1900) + 1900
    }

    pub fn month(&self) -> Option<u8> {
        self.month.map(|m| m.get())
    }

    pub fn day(&self) -> Option<u8> {
        self.day.map(|d| d.get())
    }

    pub fn pgn(self) -> impl fmt::Display {
        fmt::from_fn(move |f| {
            write!(f, "{:04}.", self.year())?;
            match self.month {
                Some(month) => write!(f, "{:02}.", month.get())?,
                None => f.write_str("??.")?,
            }
            match self.day {
                Some(day) => write!(f, "{:02}", day.get()),
                None => f.write_str("??"),
            }
        })
    }

    pub fn earliest_month(&self) -> Month {
        Month::from_months_since_jan_1900(
            u16::from(self.year_since_1900) * 12 + u16::from(self.month().unwrap_or(1)) - 1,
        )
    }

    pub fn from_ymd(year: u32, month: Option<u8>, day: Option<u8>) -> Result<Self, InvalidLaxDate> {
        Ok(LaxDate {
            year_since_1900: year
                .checked_sub(1900)
                .and_then(|year_since_1900| u8::try_from(year_since_1900).ok())
                .ok_or(InvalidLaxDate)?,
            month: month
                .map(|month| {
                    NonZeroU8::new(month)
                        .filter(|m| m.get() <= 12)
                        .ok_or(InvalidLaxDate)
                })
                .transpose()?,
            day: day
                .map(|day| {
                    NonZeroU8::new(day)
                        .filter(|d| d.get() <= 31)
                        .ok_or(InvalidLaxDate)
                })
                .transpose()?
                .filter(|_| month.is_some()),
        })
    }

    pub fn pack<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(self.year_since_1900);
        buf.put_u8(self.month.map_or(0, |m| m.get()));
        buf.put_u8(self.day.map_or(0, |d| d.get()));
    }

    pub fn unpack<B: Buf>(buf: &mut B) -> Self {
        let year_since_1900 = buf.get_u8();
        let month = NonZeroU8::new(buf.get_u8());
        assert!(month.is_none_or(|m| m.get() <= 12));
        let day = NonZeroU8::new(buf.get_u8());
        assert!(day.is_none_or(|d| month.is_some() && d.get() <= 31));

        LaxDate {
            year_since_1900,
            month,
            day,
        }
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<Self, InvalidLaxDate> {
        let mut parts = bytes.splitn(3, |&b| b == b'-' || b == b'.');
        let year_since_1900 = parts
            .next()
            .and_then(|part| btou::<u32>(part).ok())
            .and_then(|year| year.checked_sub(1900))
            .and_then(|year_since_1900| u8::try_from(year_since_1900).ok())
            .ok_or(InvalidLaxDate)?;
        let month = parts
            .next()
            .filter(|part| part != b"?" && part != b"??")
            .map(|part| {
                btou::<u8>(part)
                    .ok()
                    .filter(|&m| m <= 12)
                    .and_then(NonZeroU8::new)
                    .ok_or(InvalidLaxDate)
            })
            .transpose()?;
        let day = parts
            .next()
            .filter(|part| part != b"?" && part != b"??")
            .map(|part| {
                btou::<u8>(part)
                    .ok()
                    .filter(|&m| m <= 31)
                    .and_then(NonZeroU8::new)
                    .ok_or(InvalidLaxDate)
            })
            .transpose()?
            .filter(|_| month.is_some());

        Ok(LaxDate {
            year_since_1900,
            month,
            day,
        })
    }
}

#[derive(Error, Debug)]
#[error("invalid lax date")]
pub struct InvalidLaxDate;

impl FromStr for LaxDate {
    type Err = InvalidLaxDate;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_ascii(s.as_bytes())
    }
}

impl fmt::Display for LaxDate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (self.day, self.month) {
            (Some(day), Some(month)) => {
                write!(f, "{:04}-{:02}-{:02}", self.year(), month.get(), day.get())
            }
            (None, Some(month)) => {
                write!(f, "{:04}-{:02}", self.year(), month.get())
            }
            _ => write!(f, "{:04}", self.year()),
        }
    }
}

impl fmt::Debug for LaxDate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LaxDate({self})")
    }
}

impl Arbitrary<'_> for LaxDate {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let year_since_1900 = u.arbitrary()?;
        let month = NonZeroU8::new(u.int_in_range(0..=12)?);
        let day = NonZeroU8::new(u.int_in_range(0..=31)?).filter(|_| month.is_some());
        Ok(LaxDate {
            year_since_1900,
            month,
            day,
        })
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_str_roundtrip() {
        arbtest(|u| {
            let lax_date: LaxDate = u.arbitrary()?;

            assert_eq!(LaxDate::from_str(&lax_date.to_string()).unwrap(), lax_date);
            Ok(())
        });
    }

    #[test]
    fn test_pgn() {
        let pgn = |year, month, day| {
            LaxDate::from_ymd(year, month, day)
                .unwrap()
                .pgn()
                .to_string()
        };
        assert_eq!(pgn(2012, Some(7), Some(3)), "2012.07.03");
        assert_eq!(pgn(2012, Some(7), None), "2012.07.??");
        assert_eq!(pgn(2012, None, None), "2012.??.??");
    }

    #[test]
    fn test_pack_roundtrip() {
        arbtest(|u| {
            let lax_date: LaxDate = u.arbitrary()?;

            let mut buf = Vec::new();
            lax_date.pack(&mut buf);
            assert_eq!(LaxDate::unpack(&mut &buf[..]), lax_date);
            Ok(())
        });
    }

    #[test]
    fn test_packed_ord() {
        arbtest(|u| {
            let left: LaxDate = u.arbitrary()?;
            let right: LaxDate = u.arbitrary()?;

            let mut left_buf = Vec::new();
            left.pack(&mut left_buf);

            let mut right_buf = Vec::new();
            right.pack(&mut right_buf);

            assert_eq!(left_buf.cmp(&right_buf), left.cmp(&right));
            Ok(())
        });
    }

    #[test]
    fn test_from_parts_roundtrip() {
        arbtest(|u| {
            let lax_date: LaxDate = u.arbitrary()?;

            assert_eq!(
                LaxDate::from_ymd(lax_date.year(), lax_date.month(), lax_date.day()).unwrap(),
                lax_date
            );
            Ok(())
        });
    }

    #[test]
    fn test_from_ascii() {
        let date = LaxDate::from_ascii(b"2025.11.13").expect("valid date");
        assert_eq!(date.year(), 2025);
        assert_eq!(date.month(), Some(11));
        assert_eq!(date.day(), Some(13));
    }
}
