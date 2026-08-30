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

use soma_guest::{GuestCommand, LaunchNetwork, TerminalStatus};
use soma_kvm::x86_64::{SandboxConfig, SandboxEvidence};

use super::worker::serve;

/// The vsock context identifier every sandbox guest is given.
///
/// One VMM process owns one guest, and the identifier names the guest inside that process, so
/// it does not have to be unique across the host.
pub(super) const GUEST_CID: u32 = 3;
/// The locally administered MAC the guest sees on its one network device.
pub(super) const GUEST_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];
/// How long a cold boot has to reach an authenticated Ready.
pub(super) const BOOT_DEADLINE: Duration = Duration::from_secs(60);
/// How long the guest has to leave `KVM_RUN` after it acknowledges shutdown.
pub(super) const EXIT_GRACE: Duration = Duration::from_secs(10);

/// What the lifecycle asks a live sandbox to do.
pub(super) enum Request {
    /// Run one bounded command over the authenticated session.
    Execute(GuestCommand),
    /// Ask the guest to shut down, then finish the machine and report its evidence.
    Shutdown,
}

/// What a live sandbox reports back.
pub(super) enum Response {
    /// The sandbox reached an authenticated Ready.
    Ready,
    /// One command completed with its typed terminal status.
    Executed(Box<Completed>),
    /// The machine stopped and released everything it owned.
    Finished(Box<SandboxEvidence>),
    /// The session failed and the thread is ending.
    Failed(SessionError),
}

/// One completed command, as the portable lifecycle reports it.
pub(super) struct Completed {
    pub(super) status: TerminalStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

/// Why a session could not do what was asked.
///
/// The variants name the stage rather than carrying the underlying message, because these cross
/// a thread boundary into a failure a caller may render.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SessionError {
    /// The machine could not be created from the prepared artifacts.
    Create,
    /// The launch page could not be delivered, or the guest never consumed it.
    LaunchPage,
    /// The guest did not reach the authenticated session before the boot deadline.
    Boot,
    /// Repair or the readiness probe failed.
    Ready,
    /// A command could not be run over the session.
    Execute,
    /// The sandbox thread ended without answering.
    Gone,
}

/// A live sandbox, addressed over channels.
pub(super) struct Session {
    requests: Sender<Request>,
    responses: Receiver<Response>,
    thread: Option<JoinHandle<()>>,
}

/// Everything one sandbox needs before it can boot.
pub(super) struct Boot {
    pub(super) config: SandboxConfig,
    pub(super) generation: [u8; 32],
    pub(super) instance: [u8; 16],
    pub(super) machine: [u8; 16],
    pub(super) network: LaunchNetwork,
}

impl Session {
    /// Boots one sandbox and returns once it is authenticated and Ready.
    ///
    /// A failure here leaves no thread behind: the sandbox thread reports it and ends, and the
    /// machine it owned is finished on the way out.
    pub(super) fn launch(boot: Boot) -> Result<Self, SessionError> {
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
        };
        match session.await_response(BOOT_DEADLINE + EXIT_GRACE) {
            Ok(Response::Ready) => Ok(session),
            // A sandbox that answered anything else never reached Ready, and one that answered
            // nothing is gone; both carry the reason the thread reported.
            Ok(Response::Failed(error)) | Err(error) => Err(error),
            Ok(_) => Err(SessionError::Boot),
        }
    }

    /// Runs one bounded command and returns its typed result.
    pub(super) fn execute(
        &mut self,
        command: GuestCommand,
        deadline: Duration,
    ) -> Result<Completed, SessionError> {
        self.requests
            .send(Request::Execute(command))
            .map_err(|_| SessionError::Gone)?;
        match self.await_response(deadline)? {
            Response::Executed(completed) => Ok(*completed),
            Response::Failed(error) => Err(error),
            _ => Err(SessionError::Execute),
        }
    }

    /// Shuts the guest down and returns the machine's evidence.
    pub(super) fn shutdown(mut self) -> Result<SandboxEvidence, SessionError> {
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

    fn await_response(&mut self, within: Duration) -> Result<Response, SessionError> {
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
            let (dead, _) = channel();
            let live = std::mem::replace(&mut self.requests, dead);
            drop(live);
            self.join();
        }
    }
}
