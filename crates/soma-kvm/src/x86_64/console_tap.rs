//! A live view of one console marker while the guest is still running.
//!
//! The captured console lives inside the port bus on the vCPU thread and is returned only
//! when the machine stops, which is too late for a snapshot builder: it must know that the
//! guest agent reached its repair point while the machine is still up. The tap is the
//! narrowest thing that answers that question - one fixed needle, one instant, no buffer the
//! guest can grow.
//!
//! Matching is over the byte stream rather than over completed lines, so nothing depends on
//! where the guest's tty decided to break a line or on how many lines came before.

use std::{
    sync::{Condvar, Mutex, PoisonError},
    time::Instant,
};

/// Watches the console for one fixed line and records when it completed.
#[derive(Debug)]
pub(crate) struct ConsoleTap {
    needle: Vec<u8>,
    seen: Mutex<Option<Instant>>,
    changed: Condvar,
}

impl ConsoleTap {
    /// Watches for the first completed console line that contains `needle`.
    pub(crate) fn watching(needle: &[u8]) -> Self {
        Self {
            needle: needle.to_vec(),
            seen: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    /// The exact byte sequence the tap is watching for.
    pub(crate) fn needle(&self) -> &[u8] {
        &self.needle
    }

    /// Records that the needle completed at `at`; the first match wins.
    pub(crate) fn record(&self, at: Instant) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if seen.is_none() {
            *seen = Some(at);
            self.changed.notify_all();
        }
    }

    /// Waits until the line completes or `deadline` passes.
    pub(crate) fn wait(&self, deadline: Instant) -> Option<Instant> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(at) = *seen {
                return Some(at);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            seen = self
                .changed
                .wait_timeout(seen, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::ConsoleTap;

    #[test]
    fn the_first_match_is_recorded_once_and_later_ones_are_ignored() {
        let tap = ConsoleTap::watching(b"awaiting launch material");
        assert_eq!(tap.needle(), b"awaiting launch material");
        assert!(tap.wait(Instant::now()).is_none());
        let first = Instant::now();
        tap.record(first);
        tap.record(first + Duration::from_secs(1));
        assert_eq!(tap.wait(Instant::now()), Some(first));
    }
}
