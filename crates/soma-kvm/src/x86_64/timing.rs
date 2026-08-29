//! Monotonic per-phase timing retained as evidence.

use std::time::{Duration, Instant};

use super::error::Phase;

/// Monotonic duration of one completed phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseTiming {
    phase: Phase,
    elapsed_ns: u64,
}

impl PhaseTiming {
    /// The phase that completed.
    #[must_use]
    pub const fn phase(self) -> Phase {
        self.phase
    }

    /// Nanoseconds spent in that phase alone.
    #[must_use]
    pub const fn elapsed_ns(self) -> u64 {
        self.elapsed_ns
    }
}

pub(crate) struct Stopwatch {
    started: Instant,
    last: Instant,
    timings: Vec<PhaseTiming>,
}

impl Stopwatch {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            timings: Vec::new(),
        }
    }

    pub(crate) fn lap(&mut self, phase: Phase) {
        let now = Instant::now();
        self.timings.push(PhaseTiming {
            phase,
            elapsed_ns: saturating_ns(now.duration_since(self.last)),
        });
        self.last = now;
    }

    /// Nanoseconds from creation through the most recent lap, and the laps themselves.
    pub(crate) fn finish(self) -> (u64, Vec<PhaseTiming>) {
        (
            saturating_ns(self.last.duration_since(self.started)),
            self.timings,
        )
    }
}

pub(crate) fn saturating_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwatch_records_phases_in_order() {
        let mut clock = Stopwatch::new();
        clock.lap(Phase::Open);
        clock.lap(Phase::Probe);
        let (total, timings) = clock.finish();
        let phases: Vec<Phase> = timings.iter().map(|timing| timing.phase()).collect();
        assert_eq!(phases, [Phase::Open, Phase::Probe]);
        assert!(total >= timings.iter().map(|t| t.elapsed_ns()).sum::<u64>());
    }
}
