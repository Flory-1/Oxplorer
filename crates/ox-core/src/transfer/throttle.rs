//! Speed limit shared by all worker threads of a job.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct Throttle {
    bytes_per_sec: u64,
    /// (start of the measuring window, bytes sent since then)
    state: Mutex<(Instant, u64)>,
}

impl Throttle {
    /// Unused bandwidth can't be saved up for longer than this (e.g. after a pause).
    const MAX_CREDIT: Duration = Duration::from_millis(500);

    /// `None` when there is no limit.
    pub fn new(bytes_per_sec: u64) -> Option<Self> {
        (bytes_per_sec > 0).then(|| Self {
            bytes_per_sec,
            state: Mutex::new((Instant::now(), 0)),
        })
    }

    /// Account for `bytes` just transferred and sleep as long as needed to stay under the limit.
    pub fn consume(&self, bytes: u64) {
        let wait = {
            let mut st = self.state.lock().unwrap();
            let now = Instant::now();
            let allowed = Duration::from_secs_f64(st.1 as f64 / self.bytes_per_sec as f64);
            if now.duration_since(st.0) > allowed + Self::MAX_CREDIT {
                // We were idle (paused, waiting for a decision): start a new window
                // with at most MAX_CREDIT of unused bandwidth.
                *st = (now - Self::MAX_CREDIT, 0);
            }
            st.1 += bytes;
            let due = st.0 + Duration::from_secs_f64(st.1 as f64 / self.bytes_per_sec as f64);
            due.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            thread::sleep(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_rate() {
        let t = Throttle::new(1_000_000).unwrap();
        let start = Instant::now();
        for _ in 0..10 {
            t.consume(100_000);
        }
        // 1 MB at 1 MB/s ≈ 1 s; allow for the half-second initial credit.
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(450), "{took:?}");
        assert!(took < Duration::from_secs(3), "{took:?}");
        assert!(Throttle::new(0).is_none());
    }
}
