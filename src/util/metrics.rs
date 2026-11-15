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

/// Latency histogram, exposed in seconds.
pub struct Histogram {
    /// Non-cumulative counts, the last one for `+Inf`.
    counts: [Counter; Histogram::LEN + 1],
    sum_nanos: Counter,
}

impl Default for Histogram {
    fn default() -> Histogram {
        Histogram {
            counts: std::array::from_fn(|_| Counter::default()),
            sum_nanos: Counter::default(),
        }
    }
}

impl Histogram {
    /// 2^SCHEMA buckets per octave, like native histograms.
    const SCHEMA: u32 = 3;
    const PER_OCTAVE: u32 = 1 << Histogram::SCHEMA;
    const MIN_OCTAVE: u32 = 10;
    const MAX_OCTAVE: u32 = 34;
    const LEN: usize =
        (Histogram::PER_OCTAVE * (Histogram::MAX_OCTAVE - Histogram::MIN_OCTAVE)) as usize + 1;

    /// Upper bounds of the buckets in nanoseconds (plus `+Inf`): 2^(k/8),
    /// rounded, from 2^10 ns (1 µs, cached responses) to 2^34 ns (17 s,
    /// slow HDD reads). `histogram_quantile` interpolates within a bucket,
    /// so its error is below 9%.
    const BOUNDS: [u64; Histogram::LEN] = {
        // Fixed point: before each root, the mantissa (< 2^PER_OCTAVE) is
        // shifted again, so i + 2 * FRACTION_BITS has to stay below 128.
        const FRACTION_BITS: u32 = 60;
        let mut bounds = [0; Histogram::LEN];
        let mut k = 0;
        while k < bounds.len() {
            let i = k as u32 % Histogram::PER_OCTAVE;
            let octave = Histogram::MIN_OCTAVE + k as u32 / Histogram::PER_OCTAVE;
            // 2^(i/2^SCHEMA) by taking SCHEMA square roots of 2^i.
            let mut mantissa = 1u128 << (i + FRACTION_BITS);
            let mut s = 0;
            while s < Histogram::SCHEMA {
                mantissa = (mantissa << FRACTION_BITS).isqrt();
                s += 1;
            }
            bounds[k] =
                (((mantissa << octave) + (1 << (FRACTION_BITS - 1))) >> FRACTION_BITS) as u64;
            k += 1;
        }
        bounds
    };

    pub fn record(&self, duration: Duration) {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        self.counts[Histogram::BOUNDS.partition_point(|&max| max < nanos)].inc();
        self.sum_nanos.add(nanos);
    }
}

fn nanos_to_seconds(nanos: u64) -> f64 {
    nanos as f64 / 1e9
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
        self.histogram_family(name, help, [(&[][..], histogram)]);
    }

    /// Latency histograms with arbitrary labels (other than `le`), one per
    /// set of labels. The name should end in `_seconds`.
    pub fn histogram_family<'a>(
        &mut self,
        name: &str,
        help: &str,
        histograms: impl IntoIterator<Item = (&'a [(&'a str, &'a str)], &'a Histogram)>,
    ) {
        debug_assert!(name.ends_with("_seconds"));
        self.header(name, "histogram", help);
        let bucket_name = format!("{name}_bucket");
        let sum_name = format!("{name}_sum");
        let count_name = format!("{name}_count");
        let les: Vec<String> = Histogram::BOUNDS
            .iter()
            .map(|&max| nanos_to_seconds(max).to_string())
            .chain(["+Inf".to_owned()])
            .collect();
        for (labels, histogram) in histograms {
            debug_assert!(labels.iter().all(|&(label, _)| label != "le"));
            let mut bucket_labels = labels.to_vec();
            let mut count = 0;
            for (counter, le) in histogram.counts.iter().zip(&les) {
                count += counter.get();
                bucket_labels.push(("le", le));
                self.sample(&bucket_name, &bucket_labels, count);
                bucket_labels.pop();
            }
            self.sample(
                &sum_name,
                labels,
                nanos_to_seconds(histogram.sum_nanos.get()),
            );
            self.sample(&count_name, labels, count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_bounds() {
        let bounds = Histogram::BOUNDS;
        assert_eq!(bounds[0], 1 << Histogram::MIN_OCTAVE);
        assert_eq!(bounds[bounds.len() - 1], 1 << Histogram::MAX_OCTAVE);
        for (k, &max) in bounds.iter().enumerate() {
            let exponent = (Histogram::PER_OCTAVE * Histogram::MIN_OCTAVE) as usize + k;
            let exact = (exponent as f64 / Histogram::PER_OCTAVE as f64).exp2();
            assert_eq!(max, exact.round() as u64, "{k}");
        }
    }

    #[test]
    fn test_histogram_record() {
        let histogram = Histogram::default();
        histogram.record(Duration::ZERO);
        histogram.record(Duration::from_nanos(1024));
        histogram.record(Duration::from_nanos(1025));
        histogram.record(Duration::from_secs(60));
        assert_eq!(histogram.counts[0].get(), 2);
        assert_eq!(histogram.counts[1].get(), 1);
        assert_eq!(histogram.counts[Histogram::BOUNDS.len()].get(), 1);
        assert_eq!(histogram.sum_nanos.get(), 60_000_002_049);
    }

    #[test]
    fn test_exposition() {
        let histogram = Histogram::default();
        histogram.record(Duration::from_micros(3));
        histogram.record(Duration::from_secs(60));
        let mut out = Exposition::new();
        out.counter("requests_total", "Requests.", 7);
        out.labeled_gauge("keys", "Keys.", "cf", [("a\"b", 1), ("c", 2)]);
        out.labeled_gauge("empty", "Not written.", "cf", Vec::<(&str, u64)>::new());
        out.histogram_family(
            "read_seconds",
            "Reads.",
            [(&[("cache", "hit")][..], &histogram)],
        );
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
             opening_explorer_read_seconds_bucket{cache=\"hit\",le=\"0.000001024\"} 0\n\
             opening_explorer_read_seconds_bucket{cache=\"hit\",le=\"0.000001117\"} 0\n"
        ));
        assert!(out.contains(
            "opening_explorer_read_seconds_bucket{cache=\"hit\",le=\"0.000002896\"} 0\n\
             opening_explorer_read_seconds_bucket{cache=\"hit\",le=\"0.000003158\"} 1\n"
        ));
        assert!(out.ends_with(
            "opening_explorer_read_seconds_bucket{cache=\"hit\",le=\"17.179869184\"} 1\n\
             opening_explorer_read_seconds_bucket{cache=\"hit\",le=\"+Inf\"} 2\n\
             opening_explorer_read_seconds_sum{cache=\"hit\"} 60.000003\n\
             opening_explorer_read_seconds_count{cache=\"hit\"} 2\n"
        ));
    }
}
