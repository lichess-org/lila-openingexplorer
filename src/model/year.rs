use chrono::{DateTime, Datelike as _, Utc};
use std::convert::TryInto;

#[derive(Debug, Copy, Clone)]
pub struct AnnoLichess(pub u8);

impl AnnoLichess {
    pub const MAX: AnnoLichess = AnnoLichess(u8::MAX);

    pub fn from_year(year: u32) -> AnnoLichess {
        AnnoLichess(year.saturating_sub(2000).try_into().unwrap_or(u8::MAX))
    }

    pub fn year(self) -> u32 {
        2000 + u32::from(self.0)
    }

    pub fn from_time(time: DateTime<Utc>) -> AnnoLichess {
        match time.year_ce() {
            (true, ce) => AnnoLichess::from_year(ce),
            (false, _) => AnnoLichess(0),
        }
    }
}
