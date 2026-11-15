use std::{
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

/// A counter that threads can wait on.
#[derive(Default)]
pub struct Gauge {
    value: Mutex<u64>,
    changed: Condvar,
}

impl Gauge {
    pub fn add(&self, n: u64) {
        *self.value.lock().expect("lock gauge") += n;
        self.changed.notify_all();
    }

    pub fn sub(&self, n: u64) {
        *self.value.lock().expect("lock gauge") -= n;
        self.changed.notify_all();
    }

    /// Blocks until the value satisfies `ready`, and returns the time
    /// waited.
    pub fn wait_until(&self, ready: impl Fn(u64) -> bool) -> Duration {
        let timer = Instant::now();
        let mut value = self.value.lock().expect("lock gauge");
        while !ready(*value) {
            value = self.changed.wait(value).expect("wait gauge");
        }
        timer.elapsed()
    }
}
