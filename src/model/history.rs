use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr};

use crate::model::{Month, Stats};

pub type History = Vec<HistorySegment>;

#[serde_as]
#[derive(Serialize, Clone, Debug)]
pub struct HistorySegment {
    #[serde_as(as = "DisplayFromStr")]
    pub month: Month,
    #[serde(flatten)]
    pub stats: Stats,
}

#[derive(Debug)]
pub struct HistoryBuilder {
    segments: Vec<HistorySegment>,
    last_total: Stats,
    last_month: Option<Month>,
    until_is_none: bool,
}

impl HistoryBuilder {
    pub fn new_between(since: Option<Month>, until: Option<Month>) -> HistoryBuilder {
        HistoryBuilder {
            segments: Vec::with_capacity(128),
            last_total: Stats::default(),
            last_month: since,
            until_is_none: until.is_none(),
        }
    }

    pub fn record_difference(&mut self, month: Month, total: Stats) {
        // Fill gap.
        if let Some(mut last_month) = self.last_month {
            while last_month < month {
                self.segments.push(HistorySegment {
                    month: last_month,
                    stats: Stats::default(),
                });
                last_month = last_month.add_months_saturating(1);
            }
        }
        self.last_month = Some(month.add_months_saturating(1));

        // Add entry.
        self.segments.push(HistorySegment {
            month,
            stats: &total - &self.last_total,
        });
        self.last_total = total;
    }

    pub fn build(mut self) -> History {
        if self.until_is_none {
            // By default, omit the last month, which may not be completely
            // indexed.
            self.segments.pop();
        }

        self.segments
    }
}
