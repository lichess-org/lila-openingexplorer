use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

use crate::api::Source;

#[derive(Default)]
pub struct Metrics {
    deploy_event_sent: AtomicBool,
    hit: HitMetrics,
    slow_hit: HitMetrics,
}

impl Metrics {
    const SLOW_DURATION: Duration = Duration::from_millis(500);

    pub fn fetch_set_deploy_event_sent(&self) -> bool {
        self.deploy_event_sent.fetch_or(true, Ordering::Relaxed)
    }

    pub fn to_influx_string(&self) -> String {
        [
            self.hit.to_influx_string(""),
            self.slow_hit.to_influx_string("slow_"),
        ]
        .join(",")
    }

    pub fn inc_lichess(&self, duration: Duration, source: Option<Source>, ply: u32) {
        self.hit.inc_lichess(source, ply);
        if Metrics::SLOW_DURATION <= duration {
            self.slow_hit.inc_lichess(source, ply);
        }
    }

    pub fn inc_masters(&self, duration: Duration, source: Option<Source>, ply: u32) {
        self.hit.inc_masters(source, ply);
        if Metrics::SLOW_DURATION <= duration {
            self.slow_hit.inc_masters(source, ply);
        }
    }

    pub fn inc_player(&self, duration: Duration, done: bool, ply: u32) {
        self.hit.inc_player(done, ply);
        if Metrics::SLOW_DURATION <= duration {
            self.slow_hit.inc_player(done, ply);
        }
    }
}

#[derive(Default)]
struct HitMetrics {
    lichess_miss: AtomicU64,
    masters_miss: AtomicU64,

    source_none: AtomicU64,
    source_analysis_lichess: AtomicU64,
    source_analysis_masters: AtomicU64,
    source_analysis_player: AtomicU64,
    source_analysis_player_incomplete: AtomicU64,
    source_fishnet: AtomicU64,
    source_opening: AtomicU64,
    source_opening_crawler: AtomicU64,

    lichess_ply: PlyMetrics,
    masters_ply: PlyMetrics,
    player_ply: PlyMetrics,
}

impl HitMetrics {
    pub fn inc_lichess(&self, source: Option<Source>, ply: u32) {
        self.lichess_miss.fetch_add(1, Ordering::Relaxed);
        self.inc_source(source, &self.source_analysis_lichess);
        self.lichess_ply.inc(ply);
    }

    pub fn inc_masters(&self, source: Option<Source>, ply: u32) {
        self.masters_miss.fetch_add(1, Ordering::Relaxed);
        self.inc_source(source, &self.source_analysis_masters);
        self.masters_ply.inc(ply);
    }

    pub fn inc_player(&self, done: bool, ply: u32) {
        match done {
            false => &self.source_analysis_player_incomplete,
            true => &self.source_analysis_player,
        }
        .fetch_add(1, Ordering::Relaxed);
        self.player_ply.inc(ply);
    }

    fn inc_source(&self, source: Option<Source>, analysis_db: &AtomicU64) {
        match source {
            None => &self.source_none,
            Some(Source::Analysis | Source::Mobile) => analysis_db,
            Some(Source::Fishnet) => &self.source_fishnet,
            Some(Source::Opening) => &self.source_opening,
            Some(Source::OpeningCrawler) => &self.source_opening_crawler,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    fn to_influx_string(&self, field_prefix: &str) -> String {
        [
            format!(
                "{}source_none={}u",
                field_prefix,
                self.source_none.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_analysis_lichess={}u",
                field_prefix,
                self.source_analysis_lichess.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_analysis_masters={}u",
                field_prefix,
                self.source_analysis_masters.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_fishnet={}u",
                field_prefix,
                self.source_fishnet.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_opening={}u",
                field_prefix,
                self.source_opening.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_opening_crawler={}u",
                field_prefix,
                self.source_opening_crawler.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_analysis_player={}u",
                field_prefix,
                self.source_analysis_player.load(Ordering::Relaxed)
            ),
            format!(
                "{}source_analysis_player_incomplete={}u",
                field_prefix,
                self.source_analysis_player_incomplete
                    .load(Ordering::Relaxed)
            ),
            self.lichess_ply
                .to_influx_string(&format!("{field_prefix}lichess_ply_")),
            self.masters_ply
                .to_influx_string(&format!("{field_prefix}masters_ply_")),
            self.player_ply
                .to_influx_string(&format!("{field_prefix}player_ply_")),
        ]
        .join(",")
    }
}

#[derive(Default)]
struct PlyMetrics {
    groups: [AtomicU64; 10],
}

impl PlyMetrics {
    const GROUP_WIDTH: usize = 5;

    fn inc(&self, ply: u32) {
        if let Some(group) = self.groups.get(ply as usize / PlyMetrics::GROUP_WIDTH) {
            group.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn to_influx_string(&self, field_prefix: &str) -> String {
        self.groups
            .iter()
            .enumerate()
            .map(|(i, group)| {
                let ply = i * PlyMetrics::GROUP_WIDTH;
                let num = group.load(Ordering::Relaxed);
                format!("{field_prefix}{ply}={num}u")
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}
