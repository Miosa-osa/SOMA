//! The parent's decision whether a launched machine runs the warm command.
//!
//! The warm command runs a vCPU for as long as the command takes. One at a time that is free,
//! because the launch has already answered; fifty at once on a 32-thread host is fifty runnable
//! vCPUs sharing cores, which measured as create rising from 5-7 ms to 21 ms and a `true` command
//! from 0.8 ms to 46 ms. So past a number of launches in flight, a launch skips the warm command
//! and its first command pays its own page faults instead of everybody paying for the pile-up.
//!
//! The decision is made where the launch is claimed, because that is the one process that sees
//! every launch at once; the machine host only learns the answer on its launch request.

use std::sync::{
    OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use super::warm;

/// Names the most launches in flight at which a launch still warms.
///
/// Unset keeps every launch warming. `0` never warms. A positive number warms only while at most
/// that many launches, this one included, are in flight. `auto` is half the host's available
/// parallelism, which leaves the other half of the threads for the creates and commands
/// themselves.
const MAX_INFLIGHT: &str = "SOMA_LAUNCH_WARM_MAX_INFLIGHT";

/// When a launch warms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Limit {
    Always,
    AtMost(usize),
}

impl Limit {
    fn parse(value: Option<&str>, parallelism: usize) -> Result<Self, ()> {
        match value.map(str::trim) {
            None => Ok(Self::Always),
            Some("auto") => Ok(Self::AtMost((parallelism / 2).max(1))),
            Some(number) => number.parse().map(Self::AtMost).map_err(|_| ()),
        }
    }

    /// The configured limit, read once per process.
    ///
    /// A value that does not parse keeps every launch warming, which is what the service did
    /// before this setting existed, and says so once.
    fn configured() -> Self {
        static LIMIT: OnceLock<Limit> = OnceLock::new();
        *LIMIT.get_or_init(|| {
            let value = std::env::var(MAX_INFLIGHT).ok();
            let parallelism = std::thread::available_parallelism().map_or(1, usize::from);
            Self::parse(value.as_deref(), parallelism).unwrap_or_else(|()| {
                eprintln!(
                    "soma-local: {MAX_INFLIGHT} is not a count or `auto`; every launch warms"
                );
                Self::Always
            })
        })
    }

    const fn warms(self, in_flight: usize) -> bool {
        match self {
            Self::Always => true,
            Self::AtMost(limit) => in_flight <= limit,
        }
    }
}

/// Launches in flight, and how many of them warmed or skipped.
struct Gate {
    in_flight: AtomicUsize,
    warmed: AtomicU64,
    skipped: AtomicU64,
}

static GATE: Gate = Gate::new();

impl Gate {
    const fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            warmed: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }

    fn enter(&self) -> InFlight<'_> {
        let in_flight = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        InFlight {
            gate: self,
            in_flight,
        }
    }
}

/// One launch counted as in flight until it is dropped.
pub(super) struct InFlight<'a> {
    gate: &'a Gate,
    in_flight: usize,
}

/// Counts one launch as in flight for as long as the returned guard lives.
pub(super) fn enter() -> InFlight<'static> {
    GATE.enter()
}

impl InFlight<'_> {
    /// Whether this launch should skip the warm command, counting the decision.
    ///
    /// A host with no warm command configured runs none either way, so nothing is counted.
    pub(super) fn skip_warm(&self) -> bool {
        if !warm::configured() {
            return false;
        }
        self.decide(Limit::configured())
    }

    fn decide(&self, limit: Limit) -> bool {
        if limit.warms(self.in_flight) {
            self.gate.warmed.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let skipped = self.gate.skipped.fetch_add(1, Ordering::Relaxed) + 1;
        let warmed = self.gate.warmed.load(Ordering::Relaxed);
        // One line per skip, carrying the running totals, so a burst test reads the split from
        // the service log without a metrics surface.
        eprintln!(
            "soma-local: launch warm skipped in_flight={} warmed_total={warmed} \
             skipped_total={skipped}",
            self.in_flight
        );
        true
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.gate.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{Gate, Limit};

    #[test]
    fn unset_always_warms_and_zero_never_does() {
        let always = Limit::parse(None, 32).expect("unset is valid");
        let never = Limit::parse(Some("0"), 32).expect("zero is valid");

        assert!(always.warms(1) && always.warms(10_000));
        assert!(!never.warms(1));
    }

    #[test]
    fn a_count_warms_up_to_and_including_itself() {
        let limit = Limit::parse(Some(" 16 "), 32).expect("16 is valid");

        assert!(limit.warms(16));
        assert!(!limit.warms(17));
    }

    #[test]
    fn auto_is_half_the_parallelism_and_never_zero() {
        assert_eq!(Limit::parse(Some("auto"), 32), Ok(Limit::AtMost(16)));
        assert_eq!(Limit::parse(Some("auto"), 1), Ok(Limit::AtMost(1)));
    }

    #[test]
    fn a_malformed_value_is_refused() {
        for value in ["", "-1", "many", "1.5"] {
            assert!(Limit::parse(Some(value), 32).is_err(), "{value}");
        }
    }

    #[test]
    fn launches_past_the_limit_skip_and_every_decision_is_counted() {
        let gate = Gate::new();
        let limit = Limit::AtMost(2);

        let first = gate.enter();
        let second = gate.enter();
        let third = gate.enter();
        let skipped = [&first, &second, &third].map(|launch| launch.decide(limit));

        assert_eq!(skipped, [false, false, true]);
        assert_eq!(gate.warmed.load(Ordering::Relaxed), 2);
        assert_eq!(gate.skipped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_finished_launch_leaves_the_count() {
        let gate = Gate::new();
        let limit = Limit::AtMost(1);
        let first = gate.enter();
        drop(first);

        let next = gate.enter();

        assert!(!next.decide(limit));
        assert_eq!(gate.in_flight.load(Ordering::Relaxed), 1);
    }
}
