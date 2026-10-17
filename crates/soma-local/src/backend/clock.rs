use std::{collections::HashMap, time::Instant};

use soma::OperationId;

const MAX_TRACKED_OPERATIONS: usize = 1_024;

pub(super) struct OperationClocks {
    starts: HashMap<OperationId, Instant>,
}

impl OperationClocks {
    pub(super) fn new() -> Self {
        Self {
            starts: HashMap::new(),
        }
    }

    /// Starts an operation's clock, if nothing has started it, and reports when it started.
    ///
    /// A launch is timed by the process that was asked for it, but a hosted launch builds the
    /// machine in another process, and an `Instant` means nothing outside the process that took
    /// it. A child that only reads its own clock therefore measures from whenever it first
    /// touched the operation, which for the launch stamp is the stamp itself: the machine appears
    /// to exist at the moment it is asked to start, and everything it took to build it is charged
    /// to the readiness that follows. A child that calls this the moment it receives the launch
    /// gives the parent an origin it can place on the timeline the request was admitted on, and
    /// the two readings can be added because both are elapsed times on a clock that keeps
    /// running while the message crosses.
    pub(super) fn start(&mut self, operation_id: &OperationId) -> Instant {
        if !self.starts.contains_key(operation_id) && self.starts.len() >= MAX_TRACKED_OPERATIONS {
            self.starts.clear();
        }
        *self
            .starts
            .entry(operation_id.clone())
            .or_insert_with(Instant::now)
    }

    pub(super) fn elapsed_ns(&mut self, operation_id: &OperationId) -> u64 {
        let started = self.start(operation_id);
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation() -> OperationId {
        OperationId::new("0f1e2d3c4b5a69788796a5b4c3d2e1f0").expect("operation")
    }

    #[test]
    fn starting_early_is_what_a_later_read_measures_from() {
        // The whole point of `start`: a machine host that starts the clock when the launch
        // arrives measures the construction it then performs, and a later read of the clock
        // reports that work rather than zero.
        let mut clocks = OperationClocks::new();
        let started = clocks.start(&operation());
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(
            clocks.elapsed_ns(&operation()) >= 20_000_000,
            "a clock started before the work must report the work"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(20));
    }

    #[test]
    fn starting_again_does_not_move_the_origin() {
        // A second call must be a read, not a restart, or the first milestone to read the clock
        // would silently become the origin again.
        let mut clocks = OperationClocks::new();
        let first = clocks.start(&operation());
        let second = clocks.start(&operation());
        assert_eq!(first, second);
    }

    #[test]
    fn one_operation_does_not_start_another() {
        let mut clocks = OperationClocks::new();
        let one = OperationId::new("00000000000000000000000000000001").expect("operation");
        let two = OperationId::new("00000000000000000000000000000002").expect("operation");
        let started = clocks.start(&one);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(
            clocks.elapsed_ns(&two) < 20_000_000,
            "another is its own clock"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(20));
    }
}
