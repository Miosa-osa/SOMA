use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(1);
/// Past this many tracked keys, windows that have ended are swept on the next check.
const SWEEP_AT: usize = 4_096;

/// A per-key fixed one-second window, the same shape as the fast-lane edge's limiter.
///
/// One lock and one map update per request; the windows that have ended are swept only when the
/// map grows, so a quiet runner does no background work for it.
#[derive(Debug)]
pub struct RateLimiter {
    epoch: Instant,
    windows: Mutex<HashMap<String, (u64, u32)>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one request for `key_id` and says whether it is within `limit` for this second.
    pub fn allow(&self, key_id: &str, limit: u32, now: Instant) -> bool {
        let window = now.saturating_duration_since(self.epoch).as_secs() / WINDOW.as_secs();
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if windows.len() >= SWEEP_AT {
            windows.retain(|_, (started, _)| *started == window);
        }
        let slot = windows.entry(key_id.to_owned()).or_insert((window, 0));
        if slot.0 != window {
            *slot = (window, 0);
        }
        slot.1 = slot.1.saturating_add(1);
        slot.1 <= limit
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::RateLimiter;

    #[test]
    fn allows_up_to_the_limit_then_refuses_until_the_next_window() {
        let limiter = RateLimiter::new();
        let now = Instant::now();

        assert!((0..3).all(|_| limiter.allow("k-1", 3, now)));
        assert!(!limiter.allow("k-1", 3, now));
        assert!(limiter.allow("k-2", 3, now), "keys are counted apart");
        assert!(limiter.allow("k-1", 3, now + Duration::from_secs(1)));
    }
}
