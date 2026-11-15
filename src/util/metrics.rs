//! Metrics in the Prometheus text exposition format (version 0.0.4).
//!
//! Scrapes are free of side effects: counters and histograms only ever
//! grow (until restart), so that any number of scrapers (and humans with
//! curl) see consistent values. Rates, quantiles and maxima over a time
//! window are for the query side (`rate`, `histogram_quantile`).

use std::{
    fmt::{Display, Write as _},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

const NAMESPACE: &str = "opening_explorer_";

#[derive(Default)]
pub struct Counter(AtomicU64);

impl Counter {
    pub fn inc(&self) {
        self.add(1);
    }

    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Upper bounds of the latency buckets in microseconds (plus `+Inf`),
/// covering cached NVMe reads up to slow HDD reads.
const BUCKETS: [u64; 15] = [
    250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000,
    2_500_000, 5_000_000, 10_000_000,
];

/// Latency histogram, exposed in seconds.
#[derive(Default)]
pub struct Histogram {
    /// Non-cumulative counts, the last one for `+Inf`.
    counts: [Counter; BUCKETS.len() + 1],
    sum_micros: Counter,
}

impl Histogram {
    pub fn record(&self, duration: Duration) {
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        let bucket = BUCKETS
            .iter()
            .position(|&max| micros <= max)
            .unwrap_or(BUCKETS.len());
        self.counts[bucket].inc();
        self.sum_micros.add(micros);
    }
}

pub fn micros_to_seconds(micros: u64) -> f64 {
    micros as f64 / 1e6
}

/// Writer of metric families. Names are given without the namespace
/// prefix, and counter names end in `_total`.
#[derive(Default)]
pub struct Exposition {
    out: String,
}

impl Exposition {
    pub fn new() -> Exposition {
        Exposition::default()
    }

    pub fn finish(self) -> String {
        self.out
    }

    fn header(&mut self, name: &str, kind: &str, help: &str) {
        debug_assert!(!help.contains(['\\', '\n']));
        let _ = writeln!(self.out, "# HELP {NAMESPACE}{name} {help}");
        let _ = writeln!(self.out, "# TYPE {NAMESPACE}{name} {kind}");
    }

    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: impl Display) {
        let _ = write!(self.out, "{NAMESPACE}{name}");
        for (i, (label, value)) in labels.iter().enumerate() {
            let sep = if i == 0 { '{' } else { ',' };
            let _ = write!(self.out, "{sep}{label}=\"");
            for c in value.chars() {
                match c {
                    '\\' => self.out.push_str("\\\\"),
                    '"' => self.out.push_str("\\\""),
                    '\n' => self.out.push_str("\\n"),
                    c => self.out.push(c),
                }
            }
            self.out.push('"');
        }
        if !labels.is_empty() {
            self.out.push('}');
        }
        let _ = writeln!(self.out, " {value}");
    }

    pub fn counter(&mut self, name: &str, help: &str, value: impl Display) {
        debug_assert!(name.ends_with("_total"));
        self.header(name, "counter", help);
        self.sample(name, &[], value);
    }

    /// A counter with a single label, one sample per value of the label.
    pub fn labeled_counter<'a, V: Display>(
        &mut self,
        name: &str,
        help: &str,
        label: &str,
        samples: impl IntoIterator<Item = (&'a str, V)>,
    ) {
        debug_assert!(name.ends_with("_total"));
        self.labeled("counter", name, help, label, samples);
    }

    pub fn gauge(&mut self, name: &str, help: &str, value: impl Display) {
        self.header(name, "gauge", help);
        self.sample(name, &[], value);
    }

    /// A gauge with a single label, one sample per value of the label.
    pub fn labeled_gauge<'a, V: Display>(
        &mut self,
        name: &str,
        help: &str,
        label: &str,
        samples: impl IntoIterator<Item = (&'a str, V)>,
    ) {
        self.labeled("gauge", name, help, label, samples);
    }

    fn labeled<'a, V: Display>(
        &mut self,
        kind: &str,
        name: &str,
        help: &str,
        label: &str,
        samples: impl IntoIterator<Item = (&'a str, V)>,
    ) {
        let mut samples = samples.into_iter().peekable();
        if samples.peek().is_none() {
            return;
        }
        self.header(name, kind, help);
        for (value, sample) in samples {
            self.sample(name, &[(label, value)], sample);
        }
    }

    /// A family of samples with arbitrary labels.
    pub fn family<'a, V: Display>(
        &mut self,
        kind: &str,
        name: &str,
        help: &str,
        samples: impl IntoIterator<Item = (&'a [(&'a str, &'a str)], V)>,
    ) {
        self.header(name, kind, help);
        for (labels, sample) in samples {
            self.sample(name, labels, sample);
        }
    }

    /// A latency histogram. The name should end in `_seconds`.
    pub fn histogram(&mut self, name: &str, help: &str, histogram: &Histogram) {
        debug_assert!(name.ends_with("_seconds"));
        self.header(name, "histogram", help);
        let bucket_name = format!("{name}_bucket");
        let mut count = 0;
        for (i, counter) in histogram.counts.iter().enumerate() {
            count += counter.get();
            let le = match BUCKETS.get(i) {
                Some(&max) => micros_to_seconds(max).to_string(),
                None => "+Inf".to_owned(),
            };
            self.sample(&bucket_name, &[("le", &le)], count);
        }
        self.sample(
            &format!("{name}_sum"),
            &[],
            micros_to_seconds(histogram.sum_micros.get()),
        );
        self.sample(&format!("{name}_count"), &[], count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exposition() {
        let histogram = Histogram::default();
        histogram.record(Duration::from_micros(100));
        histogram.record(Duration::from_millis(3));
        histogram.record(Duration::from_secs(60));
        let mut out = Exposition::new();
        out.counter("requests_total", "Requests.", 7);
        out.labeled_gauge("keys", "Keys.", "cf", [("a\"b", 1), ("c", 2)]);
        out.labeled_gauge("empty", "Not written.", "cf", Vec::<(&str, u64)>::new());
        out.histogram("read_seconds", "Reads.", &histogram);
        let out = out.finish();
        assert!(out.starts_with(
            "# HELP opening_explorer_requests_total Requests.\n\
             # TYPE opening_explorer_requests_total counter\n\
             opening_explorer_requests_total 7\n\
             # HELP opening_explorer_keys Keys.\n\
             # TYPE opening_explorer_keys gauge\n\
             opening_explorer_keys{cf=\"a\\\"b\"} 1\n\
             opening_explorer_keys{cf=\"c\"} 2\n\
             # HELP opening_explorer_read_seconds Reads.\n\
             # TYPE opening_explorer_read_seconds histogram\n\
             opening_explorer_read_seconds_bucket{le=\"0.00025\"} 1\n\
             opening_explorer_read_seconds_bucket{le=\"0.0005\"} 1\n\
             opening_explorer_read_seconds_bucket{le=\"0.001\"} 1\n\
             opening_explorer_read_seconds_bucket{le=\"0.0025\"} 1\n\
             opening_explorer_read_seconds_bucket{le=\"0.005\"} 2\n"
        ));
        assert!(out.ends_with(
            "opening_explorer_read_seconds_bucket{le=\"10\"} 2\n\
             opening_explorer_read_seconds_bucket{le=\"+Inf\"} 3\n\
             opening_explorer_read_seconds_sum 60.0031\n\
             opening_explorer_read_seconds_count 3\n"
        ));
    }
}
