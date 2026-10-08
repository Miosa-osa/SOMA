//! The messages one live sandbox and its owner exchange.
//!
//! The machine and its session live on a thread of their own, so the lifecycle speaks to them
//! over these channels. Each request is one bounded act; each response reports the act or the
//! stage it failed in, because a failure crosses a thread boundary into a caller that renders it.

use soma_guest::{ActivationReceipt, GuestCommand, TerminalStatus};
use soma_kvm::x86_64::SandboxEvidence;

use super::super::sterile::Assignment;

/// What the lifecycle asks a live sandbox to do.
pub enum Request {
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
    /// Perform one bounded terminal operation over the authenticated session.
    Pty(soma::PtyOperation),
    /// Ask the guest to shut down, then finish the machine and report its evidence.
    Shutdown,
    /// End the machine without asking the guest, then report its evidence.
    ///
    /// The vCPU is kicked out of `KVM_RUN` and never re-entered, so nothing the guest does after
    /// this request is received can run. It is what a forced destroy promises, and it costs no
    /// guest poweroff: a guest asked to shut down runs its whole kernel shutdown path first.
    Abort,
}

/// What a live sandbox reports back.
pub enum Response {
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
    /// One terminal operation was performed and the guest answered it.
    PtyAnswered(Box<soma::PtyAnswer>),
    /// The machine stopped and released everything it owned.
    Finished(Box<SandboxEvidence>),
    /// An aborted machine's vCPU will never run again; its release is still under way.
    Halted,
    /// The session failed and the thread is ending.
    Failed(SessionError),
}

/// One completed command, as the portable lifecycle reports it.
pub struct Completed {
    pub status: TerminalStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Why a session could not do what was asked.
///
/// The variants name the stage rather than carrying the underlying message, because these cross
/// a thread boundary into a failure a caller may render.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionError {
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
    /// A terminal operation could not be performed over the session.
    Pty,
    /// The sandbox thread ended without answering.
    Gone,
    /// An earlier operation ended without a certain answer, so this session was ended.
    Poisoned,
}
