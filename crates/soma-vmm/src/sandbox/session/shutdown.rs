//! Ending a session the owner still holds, and what is left to wait for afterwards.
//!
//! A shutdown asks the guest to leave and collects the evidence it produced. An abort does not
//! ask: it ends the thread's command loop and leaves the slow half of the teardown running,
//! which is what `Teardown` hands back. The two are kept together here because they are the
//! only two ways an owned session stops being usable.

use std::thread::JoinHandle;

use soma_kvm::x86_64::SandboxEvidence;

use super::{BOOT_DEADLINE, EXIT_GRACE, Request, Response, Session, SessionError};

impl Session {
    /// Shuts the guest down and returns the machine's evidence.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Poisoned`] for a session already ended, or the failure that
    /// stopped the shutdown exchange.
    pub fn shutdown(mut self) -> Result<SandboxEvidence, SessionError> {
        if self.poisoned {
            // The thread is already stopped and the machine released; there is no evidence to
            // collect and no guest left to ask.
            return Err(SessionError::Poisoned);
        }
        self.requests
            .send(Request::Shutdown)
            .map_err(|_| SessionError::Gone)?;
        let evidence = match self.await_response(BOOT_DEADLINE + EXIT_GRACE) {
            Ok(Response::Finished(evidence)) => Ok(*evidence),
            Ok(Response::Failed(error)) | Err(error) => Err(error),
            Ok(_) => Err(SessionError::Gone),
        };
        self.join();
        evidence
    }

    /// Stops the machine without asking the guest, and hands back the rest of its teardown.
    ///
    /// The sandbox thread leaves its command loop and kicks the vCPU out of `KVM_RUN` without
    /// waiting for the guest to leave on its own. It answers as soon as the guest can no longer
    /// run, and only then releases the VM, its memory, and its descriptors, which is the slow
    /// half of a teardown. The returned [`Teardown`] is that half; waiting on it is how a caller
    /// learns that nothing of the machine is left.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Poisoned`] for a session already ended, whose machine is already
    /// released, or the failure that ended the thread before it answered. On an error the thread
    /// has been joined, so nothing is left to wait for.
    pub fn abort(mut self) -> Result<Teardown, SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        self.requests
            .send(Request::Abort)
            .map_err(|_| SessionError::Gone)?;
        match self.await_response(EXIT_GRACE) {
            Ok(Response::Halted) => Ok(Teardown {
                thread: self.thread.take(),
            }),
            Ok(Response::Failed(error)) | Err(error) => {
                self.join();
                Err(error)
            }
            Ok(_) => {
                self.join();
                Err(SessionError::Gone)
            }
        }
    }
}

/// The release of an aborted machine, still running on the thread that owned it.
///
/// Dropping it detaches that thread, which still finishes the release on its own; [`wait`]
/// blocks until it has.
///
/// [`wait`]: Teardown::wait
#[must_use = "an aborted machine is still being released; wait for it or let it finish detached"]
pub struct Teardown {
    thread: Option<JoinHandle<()>>,
}

impl Teardown {
    /// Blocks until the VM, its memory, and its descriptors are released.
    pub fn wait(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ignored = thread.join();
        }
    }
}
