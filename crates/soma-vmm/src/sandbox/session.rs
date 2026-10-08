//! One sandbox and its authenticated session, owned by one thread.
//!
//! A KVM sandbox outlives the call that launched it, but its session cannot be stored beside
//! the machine it talks to: the host adapter borrows the machine so that committing repair can
//! retire the launch page. A structure holding both would have to refer to itself.
//!
//! So the machine and the session live on a thread of their own, where that borrow is an
//! ordinary local one, and the lifecycle speaks to them over channels. Nothing here is shared
//! state: one thread owns the machine for its whole life, and the last command it accepts ends
//! it. That is also the shape a sandbox process eventually takes, so the seam does not move
//! when the daemon owns it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use soma_guest::ActivationReceipt;

use super::source::Boot;
use super::worker::serve;

mod protocol;
mod shutdown;

pub use protocol::{Completed, Request, Response, SessionError};
pub use shutdown::Teardown;

/// How long a cold boot has to reach an authenticated Ready.
pub const BOOT_DEADLINE: Duration = Duration::from_secs(60);
/// How long the guest has to leave `KVM_RUN` after it acknowledges shutdown.
pub const EXIT_GRACE: Duration = Duration::from_secs(10);
/// How long one whole filesystem operation has to answer.
///
/// A whole-file transfer is several bounded records rather than one, so the ceiling covers the
/// loop and not a single exchange; the guest protocol bounds each record inside it.
pub(super) const FILE_CEILING: Duration = Duration::from_secs(120);

/// How long one terminal operation has to answer.
///
/// It is the longest wait a read may ask for, plus room for the exchange around it. A terminal
/// call is one record in each direction and the only one that ever waits is a read, which states
/// its own bound; anything beyond that is a session that is not answering.
pub const PTY_CEILING: Duration = Duration::from_millis(soma::MAX_PTY_WAIT_MILLIS as u64 + 30_000);

/// A live sandbox, addressed over channels.
pub struct Session {
    pub(super) requests: Sender<Request>,
    pub(super) responses: Receiver<Response>,
    pub(super) thread: Option<JoinHandle<()>>,
    /// Set once an operation ended without a certain answer.
    ///
    /// A timed-out command may still be running, and its reply would arrive on the same channel
    /// as the next command's. Attributing it to that next command would report one command's
    /// output as another's, so an uncertain outcome ends the session instead.
    pub(super) poisoned: bool,
}

impl Session {
    /// Boots one sandbox and returns once it is authenticated and Ready.
    ///
    /// A failure here leaves no thread behind: the sandbox thread reports it and ends, and the
    /// machine it owned is finished on the way out.
    ///
    /// # Errors
    ///
    /// Returns the [`SessionError`] that stopped the sandbox before it reached Ready.
    pub fn launch(
        boot: Boot,
        activate: &mut dyn FnMut(&ActivationReceipt) -> Result<(), SessionError>,
    ) -> Result<Self, SessionError> {
        let (request_tx, request_rx) = channel();
        let (response_tx, response_rx) = channel();
        let thread = std::thread::Builder::new()
            .name("soma-kvm-sandbox".to_owned())
            .spawn(move || serve(boot, &request_rx, &response_tx))
            .map_err(|_| SessionError::Create)?;
        let mut session = Self {
            requests: request_tx,
            responses: response_rx,
            thread: Some(thread),
            poisoned: false,
        };
        match session.await_response(BOOT_DEADLINE + EXIT_GRACE) {
            Ok(Response::Ready) => Ok(session),
            Ok(Response::Minted(receipt)) => session.open_the_link(&receipt, activate),
            // A sandbox that answered anything else never reached Ready, and one that answered
            // nothing is gone; both carry the reason the thread reported.
            Ok(Response::Failed(error)) | Err(error) => Err(error),
            Ok(_) => Err(SessionError::Boot),
        }
    }

    /// Activates the assignment with the minted receipt, then raises the machine's link.
    ///
    /// The order is the whole point. The guest has repaired its interface by the time the
    /// receipt exists; the broker raises its own links, installs the routes, and enables
    /// forwarding when it accepts the receipt; and only then does the machine stop dropping
    /// frames. Raising the link any earlier would carry frames to an interface still holding
    /// the placeholder identity the Generation was captured with.
    fn open_the_link(
        mut self,
        receipt: &ActivationReceipt,
        activate: &mut dyn FnMut(&ActivationReceipt) -> Result<(), SessionError>,
    ) -> Result<Self, SessionError> {
        self.raise_the_link(receipt, activate)?;
        Ok(self)
    }

    /// Whether this session may still be used.
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        !self.poisoned
    }

    /// Records that no further operation may be attributed to this session, and ends it.
    ///
    /// The sandbox thread is stopped here rather than left running, so a command still executing
    /// behind a host timeout cannot keep a guest alive after the Backend stopped tracking it.
    pub(super) fn poison(&mut self, error: SessionError) -> SessionError {
        self.poisoned = true;
        self.stop_thread();
        error
    }

    /// Ends the sandbox thread and waits for the machine to be released.
    fn stop_thread(&mut self) {
        let (dead, _) = channel();
        drop(std::mem::replace(&mut self.requests, dead));
        self.join();
    }

    pub(super) fn await_response(&mut self, within: Duration) -> Result<Response, SessionError> {
        match self.responses.recv_timeout(within) {
            Ok(response) => Ok(response),
            // Both a timeout and a closed channel mean no answer is coming.
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                Err(SessionError::Gone)
            }
        }
    }

    fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ignored = thread.join();
        }
    }
}

impl Drop for Session {
    /// A dropped session must not leave a machine running.
    ///
    /// Dropping the request sender ends the thread's receive loop, which finishes the machine on
    /// its way out; the join then waits for the resources to be released rather than racing the
    /// process into its next operation.
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop_thread();
        }
    }
}
