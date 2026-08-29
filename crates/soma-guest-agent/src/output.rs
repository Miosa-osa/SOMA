//! Output accounting for one bounded command.
//!
//! The budget admits exactly the bytes that fit the authenticated combined allowance so the
//! terminal `OutputLimit` report matches the peer's accounting byte for byte.
//!
//! Accounting happens before any byte leaves the executor's one fixed read buffer.
//! [`OutputBudget::room`] bounds every read to the unspent allowance plus one probe byte, so a
//! hostile process can never make the agent read, copy, or queue more than that.

use soma_guest::TerminalStatus;

/// Largest chunk carried by one authenticated output record.
pub const MAX_CHUNK_BYTES: usize = 4096;

/// Byte admission against one combined stdout and stderr allowance.
#[derive(Debug)]
pub struct OutputBudget {
    allowance: u64,
    sent: u64,
}

/// The admission decision for one bounded read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reservation {
    /// Bytes that may be delivered; never more than the allowance that was still unspent.
    pub admitted: usize,
    /// Whether this read proved the command produces more output than the allowance permits.
    pub reached_limit: bool,
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

    /// Returns how many bytes the next read may consume.
    ///
    /// The value is the unspent allowance plus one probe byte, capped by the fixed chunk size,
    /// and is never zero.
    /// The probe byte is what separates a command that produced exactly the allowance from one
    /// that produced more, without ever reading a second unaccounted chunk.
    #[must_use]
    pub fn room(&self) -> usize {
        let chunk = u64::try_from(MAX_CHUNK_BYTES).unwrap_or(u64::MAX);
        let probe = self.remaining().saturating_add(1);
        usize::try_from(probe.min(chunk)).unwrap_or(MAX_CHUNK_BYTES)
    }

    /// Reserves allowance for `read` bytes that are already in the fixed buffer.
    ///
    /// Only the admitted prefix may be copied out of that buffer.
    #[must_use]
    pub fn reserve(&mut self, read: usize) -> Reservation {
        let remaining = self.remaining();
        let length = u64::try_from(read).unwrap_or(u64::MAX);
        if length <= remaining {
            self.sent = self.sent.saturating_add(length);
            return Reservation {
                admitted: read,
                reached_limit: false,
            };
        }
        let admitted = usize::try_from(remaining).unwrap_or(usize::MAX);
        self.sent = self.allowance;
        Reservation {
            admitted,
            reached_limit: true,
        }
    }

    /// Returns the unspent allowance.
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.allowance.saturating_sub(self.sent)
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

    fn admitted(reservation: Reservation) -> usize {
        assert!(!reservation.reached_limit);
        reservation.admitted
    }

    #[test]
    fn reads_are_bounded_by_the_unspent_allowance_plus_one_probe_byte() {
        let mut budget = OutputBudget::new(10);

        assert_eq!(budget.room(), 11);
        assert_eq!(admitted(budget.reserve(4)), 4);
        assert_eq!(budget.room(), 7);
        assert_eq!(admitted(budget.reserve(6)), 6);
        assert_eq!(budget.remaining(), 0);
        assert_eq!(budget.room(), 1);
        assert_eq!(budget.sent(), 10);
    }

    #[test]
    fn room_never_exceeds_the_fixed_chunk_size() {
        assert_eq!(OutputBudget::new(16 * 1024 * 1024).room(), MAX_CHUNK_BYTES);
        assert_eq!(OutputBudget::new(u64::MAX).room(), MAX_CHUNK_BYTES);
        assert_eq!(OutputBudget::new(1).room(), 2);
    }

    #[test]
    fn a_crossing_read_is_cut_to_exactly_fill_the_allowance() {
        let mut budget = OutputBudget::new(5);

        assert_eq!(admitted(budget.reserve(3)), 3);
        assert_eq!(
            budget.reserve(3),
            Reservation {
                admitted: 2,
                reached_limit: true
            }
        );
        assert_eq!(budget.sent(), 5);
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn the_probe_byte_alone_reaches_the_limit_without_being_delivered() {
        let mut budget = OutputBudget::new(4);
        assert_eq!(admitted(budget.reserve(4)), 4);

        assert_eq!(
            budget.reserve(1),
            Reservation {
                admitted: 0,
                reached_limit: true
            }
        );
    }

    #[test]
    fn output_that_exactly_fills_the_allowance_is_not_a_limit() {
        let mut budget = OutputBudget::new(4);

        assert_eq!(admitted(budget.reserve(4)), 4);
        assert_eq!(budget.sent(), 4);
        assert_eq!(budget.remaining(), 0);
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
