use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use clap::builder::RangedI64ValueParser;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};

/// Parses a zstd level, or 0 for no compression.
pub fn zstd_level() -> RangedI64ValueParser<i32> {
    clap::value_parser!(i32).range(0..=19)
}

/// A progress bar for games, with a stable rate display.
pub fn progress_bar(len: u64) -> ProgressBar {
    ProgressBar::new(len).with_style(
        ProgressStyle::with_template(
            "{spinner} {prefix} {msg} {wide_bar} {human_pos}/{human_len} {rate:>9} {eta:>7}",
        )
        .expect("progress template")
        .with_key("rate", |state: &ProgressState, w: &mut dyn fmt::Write| {
            let _ = write!(w, "{}/s", SignificantRate(state.per_sec()));
        }),
    )
}

/// Formats a rate with 3 significant digits and an SI suffix, so that the
/// progress bar does not jitter with every update.
pub struct SignificantRate(pub f64);

impl fmt::Display for SignificantRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (value, suffix) = match self.0 {
            r if r >= 1e9 => (r / 1e9, "G"),
            r if r >= 1e6 => (r / 1e6, "M"),
            r if r >= 1e3 => (r / 1e3, "k"),
            r => (r, ""),
        };
        let decimals = match value {
            v if v >= 100.0 => 0,
            v if v >= 10.0 => 1,
            _ => 2,
        };
        write!(f, "{value:.decimals$}{suffix}")
    }
}

/// Prints a line above the progress bar, prefixed with the elapsed time.
pub fn log_above(progress: &ProgressBar, msg: String) {
    progress.println(format!("[{:>8.1?}] {msg}", progress.elapsed()));
}

/// Runs `f` and adds the time it took to `counter`, in nanoseconds.
pub fn timed<T>(counter: &AtomicU64, f: impl FnOnce() -> T) -> T {
    let timer = Instant::now();
    let result = f();
    counter.fetch_add(timer.elapsed().as_nanos() as u64, Ordering::Relaxed);
    result
}

/// Seconds accumulated by [`timed`].
pub fn timed_seconds(counter: &AtomicU64) -> f64 {
    counter.load(Ordering::Relaxed) as f64 / 1e9
}
