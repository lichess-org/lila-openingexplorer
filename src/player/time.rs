use std::{
    fmt,
    time::{Duration, SystemTime},
};

use arbitrary::Arbitrary;
use bytes::Buf;
use serde::Deserialize;

use crate::packed::{Month, pack_uint, unpack_uint};

/// Milliseconds since 1970-01-01 00:00 UTC.
#[derive(
    Arbitrary, Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize,
)]
#[serde(transparent)]
pub struct UnixMillis(pub u64);

impl UnixMillis {
    pub const EPOCH: UnixMillis = UnixMillis(0);

    pub fn now() -> UnixMillis {
        UnixMillis(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        )
    }

    /// The time from `earlier` to this, or zero if `earlier` is later.
    pub fn elapsed_since(self, earlier: UnixMillis) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }

    /// The month (UTC), saturating at the representable months.
    pub fn month(self) -> Month {
        Month::from_unix_ms(self.0)
    }

    pub fn pack(self, buf: &mut Vec<u8>) {
        pack_uint(buf, self.0);
    }

    pub fn unpack<B: Buf>(buf: &mut B) -> UnixMillis {
        UnixMillis(unpack_uint(buf))
    }
}

impl fmt::Display for UnixMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_elapsed_since() {
        assert_eq!(
            UnixMillis(1500).elapsed_since(UnixMillis(1000)),
            Duration::from_millis(500)
        );
        assert_eq!(
            UnixMillis(1000).elapsed_since(UnixMillis(1500)),
            Duration::ZERO
        );
    }

    #[test]
    fn test_month() {
        // lastMoveAt of a lila export: 2022-04-28 07:45:35 UTC.
        assert_eq!(UnixMillis(1_651_131_935_030).month().to_string(), "2022-04");
    }
}
