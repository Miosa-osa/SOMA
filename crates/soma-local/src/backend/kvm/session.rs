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

use soma_guest::{ActivationReceipt, GuestCommand, LaunchNetwork, SecretFile, TerminalStatus};
use soma_kvm::x86_64::{NetworkAttachment, SandboxEvidence};

use super::network::PendingActivation;
use super::sterile::Assignment;
use super::worker::serve;

/// The locally administered MAC the guest sees on its one network device.
pub(super) const GUEST_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];
/// How long a cold boot has to reach an authenticated Ready.
pub(super) const BOOT_DEADLINE: Duration = Duration::from_secs(60);
/// How long the guest has to leave `KVM_RUN` after it acknowledges shutdown.
pub(super) const EXIT_GRACE: Duration = Duration::from_secs(10);
/// How long one whole filesystem operation has to answer.
///
/// A whole-file transfer is several bounded records rather than one, so the ceiling covers the
/// loop and not a single exchange; the guest protocol bounds each record inside it.
pub(super) const FILE_CEILING: Duration = Duration::from_secs(120);

/// What the lifecycle asks a live sandbox to do.
pub(super) enum Request {
    /// Transfer fresh Instance authority into a parked sterile machine, exactly once.
    ///
    /// The assignment is boxed because it is far larger than the other requests and only one
    /// sandbox in its whole life ever receives it.
    Assign(Box<Assignment>),
    /// Raise the machine's link gate, now that the broker has activated the assignment.
    RaiseLink,
    /// Run one bounded command over the authenticated session.
    Execute(GuestCommand),
    /// Perform one bounded filesystem operation over the authenticated session.
    File(soma::FileOperation),
    /// Ask the guest to shut down, then finish the machine and report its evidence.
    Shutdown,
}

/// What a live sandbox reports back.
pub(super) enum Response {
    /// The machine is restored, holds no Instance authority, and is parked to be claimed.
    Prepared,
    /// The repaired session minted the capability the broker's activation requires.
    ///
    /// The receipt can only be minted from inside the session, and activation can only be
    /// requested by the peer that claimed the assignment, which is the owner of this Session.
    /// So the two halves meet here: the sandbox thread mints and waits, the owner activates,
    /// and only then is the link raised.
    Minted(Box<ActivationReceipt>),
    /// The sandbox reached an authenticated Ready.
    Ready,
    /// One command completed with its typed terminal status.
    Executed(Box<Completed>),
    /// One filesystem operation was performed and the guest answered it.
    FileAnswered(Box<soma::FileAnswer>),
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
    /// The assigned network could not be activated, so no traffic may flow.
    Network,
    /// The guest did not reach the authenticated session before the boot deadline.
    Boot,
    /// Repair or the readiness probe failed.
    Ready,
    /// A secret this Instance was launched with could not be placed inside it.
    Secret,
    /// A command could not be run over the session.
    Execute,
    /// A filesystem operation could not be performed over the session.
    File,
    /// The sandbox thread ended without answering.
    Gone,
    /// An earlier operation ended without a certain answer, so this session was ended.
    Poisoned,
}

/// A live sandbox, addressed over channels.
pub(super) struct Session {
    requests: Sender<Request>,
    responses: Receiver<Response>,
    thread: Option<JoinHandle<()>>,
    /// Set once an operation ended without a certain answer.
    ///
    /// A timed-out command may still be running, and its reply would arrive on the same channel
    /// as the next command's. Attributing it to that next command would report one command's
    /// output as another's, so an uncertain outcome ends the session instead.
    poisoned: bool,
}

#[path = "session/assign.rs"]
mod assign;

/// The two bounded operations a live session answers.
#[path = "session/operations.rs"]
mod operations;

/// Everything one sandbox needs before it can boot.
#[path = "session/source.rs"]
mod source;
pub(in crate::backend::kvm) use source::Source;

pub(super) struct Boot {
    /// How this sandbox comes into existence.
    pub(super) source: Source,
    pub(super) generation: [u8; 32],
    pub(super) instance: [u8; 16],
    /// The operation this launch belongs to, bound into the launch page.
    pub(super) operation: [u8; 16],
    /// The vsock context identifier this Instance is assigned.
    ///
    /// Context identifiers are host global, so every concurrent sandbox needs its own. One
    /// command line invocation serves one sandbox, so there is no shared counter to draw from
    /// and the identifier is derived from the Instance identity instead.
    pub(super) guest_cid: u32,
    /// The network this Instance was given.
    pub(super) network: Network,
    /// The secrets this one Instance is launched with.
    ///
    /// They belong to the Boot rather than to the Generation because the Generation, its
    /// artifacts, and the snapshot every Instance of it restores from are shared. A value placed
    /// here reaches one machine over one session and is never part of anything a second Instance
    /// can read.
    pub(super) secrets: Vec<SecretFile>,
}

/// The network one machine is built with.
///
/// The launch values are always present, because the guest repairs an interface either way; the
/// frame path and the activation are present only for an Instance the broker leased a bundle to.
pub(super) struct Network {
    /// The values the launch page carries.
    pub(super) launch: LaunchNetwork,
    /// The assigned frame path, attached with the link still down.
    pub(super) attachment: Option<NetworkAttachment>,
    /// What the repaired session must mint before the broker will let traffic flow.
    pub(super) activation: Option<PendingActivation>,
}

impl Session {
    /// Boots one sandbox and returns once it is authenticated and Ready.
    ///
    /// A failure here leaves no thread behind: the sandbox thread reports it and ends, and the
    /// machine it owned is finished on the way out.
    pub(super) fn launch(
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
    pub(super) const fn is_usable(&self) -> bool {
        !self.poisoned
    }

    /// Records that no further operation may be attributed to this session, and ends it.
    ///
    /// The sandbox thread is stopped here rather than left running, so a command still executing
    /// behind a host timeout cannot keep a guest alive after the Backend stopped tracking it.
    fn poison(&mut self, error: SessionError) -> SessionError {
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

    /// Shuts the guest down and returns the machine's evidence.
    pub(super) fn shutdown(mut self) -> Result<SandboxEvidence, SessionError> {
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
            self.stop_thread();
        }
    }
}
