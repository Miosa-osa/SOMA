//! The guest side of the interactive terminal protocol.
//!
//! The agent holds at most one terminal session at a time, and that is a decision rather than an
//! omission. The authenticated control channel is strictly serial: one request is in flight, its
//! one answer returns the owner to idle, and there is no way for the guest to speak unprompted.
//! A second session would therefore buy no concurrency at all, only an identifier in every
//! message, a table to look it up in, and a policy for which session a read drains first. One
//! sandbox is one workspace, so one terminal is the honest shape; a caller that genuinely needs
//! two shells starts two sandboxes, which is what isolation is for.
//!
//! Requests arrive already decoded, so dimensions are within the bounded grid and a write chunk
//! is within what one record carries. What remains is the part the protocol left to the agent:
//! allocating a real pseudo-terminal, running the shell as its session leader, and moving bytes.
//!
//! No failure carries an errno or a message, for the same reason no filesystem failure does.

mod device;
mod session;

#[cfg(test)]
mod tests;

use soma_guest::{PtyFailure, PtyOutcome, PtyRequest, PtySize};

use session::Session;

/// The agent's single terminal slot.
#[derive(Default)]
pub struct Terminal {
    session: Option<Session>,
}

impl Terminal {
    /// Creates the slot with no session in it.
    #[must_use]
    pub const fn new() -> Self {
        Self { session: None }
    }

    /// Performs one decoded terminal request and returns the outcome that answers it.
    pub fn perform(&mut self, request: &PtyRequest) -> PtyOutcome {
        match request {
            PtyRequest::Open(size) => self.open(*size),
            PtyRequest::Write { bytes } => self.write(bytes),
            PtyRequest::Read { wait_millis } => self.read(*wait_millis),
            PtyRequest::Resize(size) => self.resize(*size),
            PtyRequest::Close => self.close(),
        }
    }

    /// Opens the one session, refusing a second while the first is alive.
    fn open(&mut self, size: PtySize) -> PtyOutcome {
        if self.session.is_some() {
            return PtyOutcome::Failed(PtyFailure::AlreadyOpen);
        }
        match Session::open(size) {
            Ok(session) => {
                self.session = Some(session);
                PtyOutcome::Opened(size)
            }
            // A terminal the guest could not start at all is a refusal rather than an
            // unclassified failure: there is no session, and the caller's next move is the same
            // whichever kernel call declined.
            Err(()) => PtyOutcome::Failed(PtyFailure::Denied),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> PtyOutcome {
        let Some(session) = self.session.as_mut() else {
            return PtyOutcome::Failed(PtyFailure::NoSession);
        };
        session.write(bytes)
    }

    /// Reads one bounded chunk, and forgets the session once it has reported its end.
    ///
    /// The end flag is the caller's signal that no further byte will ever arrive, so keeping a
    /// dead session in the slot afterwards would let a caller read a terminal that no longer
    /// exists and be told nothing was ready. Dropping it here makes the next request name the
    /// truth: there is no session.
    fn read(&mut self, wait_millis: u32) -> PtyOutcome {
        let Some(session) = self.session.as_mut() else {
            return PtyOutcome::Failed(PtyFailure::NoSession);
        };
        let outcome = session.read(wait_millis);
        if matches!(outcome, PtyOutcome::Output { end: true, .. }) {
            self.session = None;
        }
        outcome
    }

    fn resize(&mut self, size: PtySize) -> PtyOutcome {
        let Some(session) = self.session.as_mut() else {
            return PtyOutcome::Failed(PtyFailure::NoSession);
        };
        session.resize(size)
    }

    fn close(&mut self) -> PtyOutcome {
        match self.session.take() {
            Some(session) => {
                session.end();
                PtyOutcome::Closed
            }
            None => PtyOutcome::Failed(PtyFailure::NoSession),
        }
    }
}
