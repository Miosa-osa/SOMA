//! Output accounting for one bounded command.
//!
//! The budget admits exactly the bytes that fit the authenticated combined allowance so the
//! terminal `OutputLimit` report matches the peer's accounting byte for byte.

use soma_guest::TerminalStatus;

/// Largest chunk carried by one authenticated output record.
pub const MAX_CHUNK_BYTES: usize = 4096;

/// Byte admission against one combined stdout and stderr allowance.
#[derive(Debug)]
pub struct OutputBudget {
    allowance: u64,
    sent: u64,
}

/// Decision for one candidate chunk.
#[derive(Debug, Eq, PartialEq)]
pub enum Admission {
    /// The complete chunk fits the remaining allowance.
    Admitted(Vec<u8>),
    /// Only this prefix fits; sending it exhausts the allowance exactly.
    Limit(Vec<u8>),
    /// The allowance was already exhausted and nothing may be sent.
    Exhausted,
}

/// How the child process ended before output precedence is applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    /// Normal exit with a status code.
    Exited(i32),
    /// Termination by a Linux signal.
    Signaled(u8),
    /// The program could not be executed.
    ExecFailed(i32),
    /// The agent could not observe a valid ending.
    Unknown,
}

impl OutputBudget {
    /// Creates a budget for one combined allowance.
    #[must_use]
    pub const fn new(allowance: u64) -> Self {
        Self { allowance, sent: 0 }
    }

    /// Admits as much of `bytes` as the remaining allowance permits.
    #[must_use]
    pub fn admit(&mut self, mut bytes: Vec<u8>) -> Admission {
        let remaining = self.allowance.saturating_sub(self.sent);
        if remaining == 0 {
            return Admission::Exhausted;
        }
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if length <= remaining {
            self.sent = self.sent.saturating_add(length);
            return Admission::Admitted(bytes);
        }
        let keep = usize::try_from(remaining).unwrap_or(usize::MAX);
        bytes.truncate(keep);
        self.sent = self.allowance;
        Admission::Limit(bytes)
    }

    /// Returns whether the allowance is exactly exhausted.
    #[cfg(test)]
    pub const fn exhausted(&self) -> bool {
        self.sent >= self.allowance
    }

    /// Returns the bytes admitted so far.
    #[cfg(test)]
    pub const fn sent(&self) -> u64 {
        self.sent
    }
}

/// Maps the observed ending onto the protocol status with output-limit and deadline precedence.
#[must_use]
pub fn terminal_status(limit_hit: bool, timed_out: bool, ending: Ending) -> TerminalStatus {
    if limit_hit {
        return TerminalStatus::OutputLimit;
    }
    if timed_out {
        return TerminalStatus::TimedOut;
    }
    match ending {
        Ending::Exited(code @ 0..=255) => TerminalStatus::Exited(code),
        Ending::Signaled(signal @ 1..=64) => TerminalStatus::Signaled(signal),
        Ending::ExecFailed(errno @ 1..=4095) => TerminalStatus::ExecFailed(errno),
        Ending::Exited(_) | Ending::Signaled(_) | Ending::ExecFailed(_) | Ending::Unknown => {
            TerminalStatus::AgentFailed(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_whole_chunks_until_the_allowance_is_reached() {
        let mut budget = OutputBudget::new(10);

        assert_eq!(budget.admit(vec![1; 4]), Admission::Admitted(vec![1; 4]));
        assert_eq!(budget.admit(vec![2; 6]), Admission::Admitted(vec![2; 6]));
        assert!(budget.exhausted());
        assert_eq!(budget.admit(vec![3; 1]), Admission::Exhausted);
        assert_eq!(budget.sent(), 10);
    }

    #[test]
    fn a_crossing_chunk_is_cut_to_exactly_fill_the_allowance() {
        let mut budget = OutputBudget::new(5);

        assert_eq!(budget.admit(vec![1; 3]), Admission::Admitted(vec![1; 3]));
        assert_eq!(budget.admit(vec![2; 9]), Admission::Limit(vec![2; 2]));
        assert!(budget.exhausted());
        assert_eq!(budget.sent(), 5);
        assert_eq!(budget.admit(vec![9; 1]), Admission::Exhausted);
    }

    #[test]
    fn a_crossing_chunk_with_no_room_left_yields_an_empty_limit_prefix() {
        let mut budget = OutputBudget::new(1);
        assert_eq!(budget.admit(vec![1; 1]), Admission::Admitted(vec![1; 1]));
        assert_eq!(budget.admit(vec![2; 2]), Admission::Exhausted);

        let mut budget = OutputBudget::new(2);
        assert_eq!(budget.admit(vec![1; 5]), Admission::Limit(vec![1; 2]));
    }

    #[test]
    fn terminal_precedence_is_limit_then_deadline_then_ending() {
        assert_eq!(
            terminal_status(true, true, Ending::Exited(0)),
            TerminalStatus::OutputLimit
        );
        assert_eq!(
            terminal_status(false, true, Ending::Signaled(9)),
            TerminalStatus::TimedOut
        );
        assert_eq!(
            terminal_status(false, false, Ending::Exited(3)),
            TerminalStatus::Exited(3)
        );
        assert_eq!(
            terminal_status(false, false, Ending::Signaled(9)),
            TerminalStatus::Signaled(9)
        );
        assert_eq!(
            terminal_status(false, false, Ending::ExecFailed(2)),
            TerminalStatus::ExecFailed(2)
        );
    }

    #[test]
    fn out_of_range_endings_become_agent_failures() {
        assert_eq!(
            terminal_status(false, false, Ending::Exited(300)),
            TerminalStatus::AgentFailed(1)
        );
        assert_eq!(
            terminal_status(false, false, Ending::Signaled(0)),
            TerminalStatus::AgentFailed(1)
        );
        assert_eq!(
            terminal_status(false, false, Ending::ExecFailed(0)),
            TerminalStatus::AgentFailed(1)
        );
        assert_eq!(
            terminal_status(false, false, Ending::Unknown),
            TerminalStatus::AgentFailed(1)
        );
    }
}
