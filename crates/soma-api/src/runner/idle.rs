//! A sandbox's idle timeout and lifetime cap (contract C7).

use std::time::{Duration, Instant};

/// The idle timeout when neither the request nor the tenant names one.
pub const DEFAULT_IDLE_TIMEOUT_SECONDS: u64 = 300;
/// The longest idle timeout a request may ask for.
pub const MAX_IDLE_TIMEOUT_SECONDS: u64 = 86_400;

/// How long a sandbox may live: an idle timeout every authenticated call resets, and a hard
/// cap from the tenant's `max_lifetime_s`. `None` means no limit of that kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifetime {
    pub idle: Option<Duration>,
    pub max: Option<Duration>,
}

impl Lifetime {
    /// `idle_seconds` of 0 is no idle timeout (C7); the cap still applies.
    #[must_use]
    pub fn from_seconds(idle_seconds: u64, max_seconds: Option<u64>) -> Self {
        Self {
            idle: (idle_seconds > 0).then(|| Duration::from_secs(idle_seconds)),
            max: max_seconds.map(Duration::from_secs),
        }
    }
}

/// One sandbox's running clock.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    idle: Option<Duration>,
    last_activity: Instant,
    hard_deadline: Option<Instant>,
}

impl Clock {
    #[must_use]
    pub fn start(lifetime: Lifetime, created: Instant) -> Self {
        Self {
            idle: lifetime.idle,
            last_activity: created,
            hard_deadline: lifetime.max.map(|max| created + max),
        }
    }

    /// Resets the idle timer: called for every authenticated call on the sandbox.
    pub fn touch(&mut self, now: Instant) {
        self.last_activity = self.last_activity.max(now);
    }

    /// Replaces the idle timeout and resets the timer (`PATCH {"timeout": N}`).
    pub fn extend(&mut self, idle: Option<Duration>, now: Instant) {
        self.idle = idle;
        self.touch(now);
    }

    /// When the sandbox ends unless touched again, or `None` if nothing will end it.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        let idle = self.idle.map(|idle| self.last_activity + idle);
        match (idle, self.hard_deadline) {
            (Some(idle), Some(hard)) => Some(idle.min(hard)),
            (idle, hard) => idle.or(hard),
        }
    }

    #[must_use]
    pub fn expired(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|deadline| deadline <= now)
    }

    #[must_use]
    pub fn idle_seconds(&self) -> u64 {
        self.idle.map_or(0, |idle| idle.as_secs())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Clock, Lifetime};

    #[test]
    fn activity_pushes_the_idle_deadline_but_not_the_cap() {
        let start = Instant::now();
        let mut clock = Clock::start(Lifetime::from_seconds(10, Some(25)), start);

        assert!(!clock.expired(start + Duration::from_secs(9)));
        assert!(clock.expired(start + Duration::from_secs(10)));
        clock.touch(start + Duration::from_secs(9));
        assert!(!clock.expired(start + Duration::from_secs(18)));
        clock.touch(start + Duration::from_secs(18));
        assert!(
            clock.expired(start + Duration::from_secs(25)),
            "the cap holds"
        );
    }

    #[test]
    fn zero_is_no_idle_timeout_and_extend_replaces_it() {
        let start = Instant::now();
        let mut clock = Clock::start(Lifetime::from_seconds(0, None), start);

        assert_eq!(clock.deadline(), None);
        assert!(!clock.expired(start + Duration::from_secs(1_000_000)));
        clock.extend(
            Some(Duration::from_secs(5)),
            start + Duration::from_secs(100),
        );
        assert_eq!(clock.idle_seconds(), 5);
        assert!(clock.expired(start + Duration::from_secs(105)));
    }
}
