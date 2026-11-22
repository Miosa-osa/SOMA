//! Suspending a session's sandbox (ADR 0034): the guest retires its session and parks, the
//! machine is read into a Suspension's plaintext members, and the machine is released.
//!
//! A refusal changes nothing: a guest below API level 4, or a cold-booted machine, is answered
//! before anything reaches it, and the session stays exactly as usable as it was. Anything else
//! ends the session, because after an acknowledged Suspend the guest no longer holds it.

use std::time::Duration;

use soma_kvm::x86_64::CaptureOutcome;

use super::{Request, Response, Session, SessionError, SuspendTarget};

/// How long a suspension may take: the acknowledgement, the pause, and reading every byte of
/// guest memory and the private head.
const SUSPEND_CEILING: Duration = Duration::from_secs(660);

/// What a suspension request came to.
#[derive(Debug)]
pub enum Suspension {
    /// The machine was read into its members and released; this session has ended.
    Captured(Box<CaptureOutcome>),
    /// Refused with nothing sent; the session is untouched and still usable.
    Refused,
}

impl Session {
    /// Suspends this sandbox into `target`, or reports that it cannot be suspended.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Poisoned`] for a session already ended, or the failure that ended
    /// this one during the suspension; the machine is released either way.
    pub fn suspend(&mut self, target: SuspendTarget) -> Result<Suspension, SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        // An activation still in flight finishes first, so the thread is in its command loop.
        self.settle()?;
        self.requests
            .send(Request::Suspend(Box::new(target)))
            .map_err(|_| SessionError::Gone)?;
        match self.await_response(SUSPEND_CEILING) {
            Ok(Response::SuspendRefused) => Ok(Suspension::Refused),
            Ok(Response::Suspended(captured)) => {
                self.poisoned = true;
                self.join();
                Ok(Suspension::Captured(captured))
            }
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Gone)),
        }
    }
}
