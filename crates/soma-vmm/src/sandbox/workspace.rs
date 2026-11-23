//! The session operation for the declared durable workspace slot.

use std::fs::File;
use std::time::Duration;

use soma_guest::WorkspaceOutcome;

use super::session::{Request, Response};
use super::{Session, SessionError};

/// A live change to an existing workspace slot.
pub enum WorkspaceChange {
    /// Unmount the guest workspace and release the backing file and its lock.
    Detach,
    /// Bind an exclusively locked writable volume with exactly the slot's declared capacity.
    Attach(File),
}

impl Session {
    /// Changes the live workspace through the worker that exclusively owns the machine.
    ///
    /// An Attach descriptor must come from the trusted volume store and retain its exclusive
    /// lock across descriptor transfer. An Attach requires a preceding successful Detach.
    ///
    /// # Errors
    ///
    /// An uncertain handoff terminates the session. A paused guest is left paused and untouched.
    pub fn change_workspace(
        &mut self,
        change: WorkspaceChange,
    ) -> Result<WorkspaceOutcome, SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        self.settle()?;
        self.requests
            .send(Request::Workspace(change))
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(Duration::from_secs(90)) {
            Ok(Response::Workspace(outcome)) => Ok(outcome),
            Ok(Response::GuestPaused) => Err(SessionError::GuestPaused),
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Workspace)),
        }
    }
}
