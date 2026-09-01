//! The two bounded operations a live session answers once it is Ready.
//!
//! They are apart from the session's own lifecycle because they are the only methods a caller
//! repeats: reaching Ready and shutting down each happen once and decide whether the sandbox
//! exists, while these run against a sandbox that already does.
//!
//! Both share one rule, which is the reason they are written the same way. An operation that did
//! not certainly produce its own answer poisons the session, because the reply to a request that
//! timed out arrives on the same channel as the next request's, and attributing one operation's
//! answer to another is the single outcome neither may have.

use std::time::Duration;

use soma_guest::GuestCommand;

use super::{Completed, FILE_CEILING, PTY_CEILING, Request, Response, Session, SessionError};

impl Session {
    /// Runs one bounded command and returns its typed result.
    pub(in crate::backend::kvm) fn execute(
        &mut self,
        command: GuestCommand,
        deadline: Duration,
    ) -> Result<Completed, SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        self.requests
            .send(Request::Execute(command))
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(deadline) {
            Ok(Response::Executed(completed)) => Ok(*completed),
            // Every other outcome leaves the answer uncertain, so the session ends carrying the
            // reason: a reported failure, an unexpected reply, or no reply at all.
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Execute)),
        }
    }

    /// Performs one bounded filesystem operation and returns the guest's answer.
    ///
    /// A filesystem operation is bounded by the session's own file deadline rather than the
    /// command ceiling: it runs no program, so the time a command may take says nothing about it.
    pub(in crate::backend::kvm) fn file(
        &mut self,
        operation: soma::FileOperation,
    ) -> Result<soma::FileAnswer, SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        self.requests
            .send(Request::File(operation))
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(FILE_CEILING) {
            Ok(Response::FileAnswered(answer)) => Ok(*answer),
            // Every other outcome leaves the answer uncertain, and an uncertain filesystem answer
            // is one a caller must not attribute to the request it made, so the session ends.
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::File)),
        }
    }

    /// Performs one bounded terminal operation and returns the guest's answer.
    ///
    /// The session state a terminal has lives entirely in the guest, so this method holds nothing
    /// between calls; a second process asking for the next read reaches the same open terminal
    /// because the guest still has it, not because anything here remembered.
    pub(in crate::backend::kvm) fn pty(
        &mut self,
        operation: soma::PtyOperation,
    ) -> Result<soma::PtyAnswer, SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        self.requests
            .send(Request::Pty(operation))
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(PTY_CEILING) {
            Ok(Response::PtyAnswered(answer)) => Ok(*answer),
            // An uncertain terminal answer is one a caller must not attribute to the request it
            // made: the next read would otherwise carry the previous one's output.
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Pty)),
        }
    }
}
