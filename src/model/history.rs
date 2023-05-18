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
}

impl HistoryBuilder {
    pub fn new_starting_at(month: Option<Month>) -> HistoryBuilder {
        HistoryBuilder {
            segments: Vec::with_capacity(128),
            last_total: Stats::default(),
            last_month: month,
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

    pub fn build(self) -> History {
        self.segments
    }
}
