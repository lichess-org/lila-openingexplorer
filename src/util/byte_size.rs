use std::{fmt, num::ParseIntError, str::FromStr};

use thiserror::Error;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteSize(pub u64);

#[derive(Debug, Error)]
pub enum ParseByteSizeError {
    #[error("invalid number: {0}")]
    Number(#[from] ParseIntError),
    #[error("unknown unit {0:?}, expected k, m, g or t")]
    Unit(String),
    #[error("too large")]
    Overflow,
}

impl FromStr for ByteSize {
    type Err = ParseByteSizeError;

    fn from_str(s: &str) -> Result<ByteSize, ParseByteSizeError> {
        let mid = s
            .bytes()
            .position(|b| !b.is_ascii_digit())
            .unwrap_or(s.len());
        let (number, unit) = s.split_at(mid);
        let shift = match unit.trim_start().to_ascii_lowercase().as_str() {
            "" | "b" => 0,
            "k" | "kb" | "kib" => 10,
            "m" | "mb" | "mib" => 20,
            "g" | "gb" | "gib" => 30,
            "t" | "tb" | "tib" => 40,
            _ => return Err(ParseByteSizeError::Unit(unit.to_owned())),
        };
        number
            .parse::<u64>()?
            .checked_mul(1 << shift)
            .map(ByteSize)
            .ok_or(ParseByteSizeError::Overflow)
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (unit, shift) in [("t", 40), ("g", 30), ("m", 20), ("k", 10)] {
            if self.0 != 0 && self.0.trailing_zeros() >= shift {
                return write!(f, "{}{unit}", self.0 >> shift);
            }
        }
        write!(f, "{}", self.0)
    }
}

impl From<ByteSize> for u64 {
    fn from(size: ByteSize) -> u64 {
        size.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        for (s, bytes) in [
            ("0", 0),
            ("4096", 4096),
            ("1k", 1 << 10),
            ("16M", 16 << 20),
            ("16MiB", 16 << 20),
            ("60g", 60 << 30),
            ("60 GB", 60 << 30),
            ("2t", 2 << 40),
            ("12b", 12),
        ] {
            assert_eq!(s.parse::<ByteSize>().unwrap(), ByteSize(bytes), "{s}");
        }

        for s in [
            "",
            "g",
            "1.5g",
            "-1",
            "1x",
            "1gg",
            "99999999999t",
            "1ü",
            "ü1",
            "1 µb",
        ] {
            assert!(s.parse::<ByteSize>().is_err(), "{s}");
        }
    }

    #[test]
    fn test_display() {
        assert_eq!(ByteSize(0).to_string(), "0");
        assert_eq!(ByteSize(16 << 20).to_string(), "16m");
        assert_eq!(ByteSize((1 << 30) + (1 << 10)).to_string(), "1048577k");
    }
}
