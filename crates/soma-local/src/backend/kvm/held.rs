//! Who holds the machine one Instance is running on.
//!
//! There are two answers and the lifecycle above must not care which. Either this process holds
//! the machine on a thread of its own, which is the one-shot shape a `soma run` needs and the
//! only shape that existed before, or a jailed worker holds it and this process addresses it
//! over a pre-connected control socket.
//!
//! The second is the one that closes the gap: a machine in a jail has no socket, no filesystem,
//! no procfs, no capabilities, an ephemeral identity in its own user namespace, and a seccomp
//! filter that kills every syscall a server needs. The first remains because a host that cannot
//! build a jail must be able to say so rather than silently serve one lifecycle as the other.

use std::time::Duration;

use soma::{
    BackendFailureKind, CleanupMethod, FileAnswer, FileOperation, InstanceId, PtyAnswer,
    PtyOperation,
};
use soma_guest::GuestCommand;
use soma_kvm::x86_64::{GuestExit, SandboxEvidence};
use soma_vmm::sandbox::{Completed, Session, dump_timeline};

use super::jailed::Jailed;
use super::start::failure_kind;

/// The machine one live Instance is running on.
pub(super) enum Held {
    /// This process holds it, on the sandbox thread that owns it for its whole life.
    Resident(Session),
    /// A jailed worker holds it, and this process is its supervisor.
    Jailed(Box<Jailed>),
}

impl Held {
    /// Whether this machine may still be addressed.
    pub(super) const fn is_usable(&self) -> bool {
        match self {
            Self::Resident(session) => session.is_usable(),
            Self::Jailed(jailed) => jailed.is_usable(),
        }
    }

    /// Runs one bounded command on the machine, wherever it is.
    pub(super) fn execute(
        &mut self,
        command: GuestCommand,
        deadline: Duration,
    ) -> Result<Completed, BackendFailureKind> {
        match self {
            Self::Resident(session) => session.execute(command, deadline).map_err(failure_kind),
            Self::Jailed(jailed) => jailed.execute(&command),
        }
    }

    /// Performs one filesystem operation on the machine, wherever it is.
    pub(super) fn file(
        &mut self,
        operation: FileOperation,
    ) -> Result<FileAnswer, BackendFailureKind> {
        match self {
            Self::Resident(session) => session.file(operation).map_err(failure_kind),
            // The jailed control protocol does not carry portable filesystem requests yet.
            Self::Jailed(_) => Err(BackendFailureKind::Unsupported),
        }
    }

    /// Performs one terminal operation on the machine, wherever it is.
    pub(super) fn pty(&mut self, operation: PtyOperation) -> Result<PtyAnswer, BackendFailureKind> {
        match self {
            Self::Resident(session) => session.pty(operation).map_err(failure_kind),
            // The jailed control protocol does not carry portable terminal requests yet.
            Self::Jailed(_) => Err(BackendFailureKind::Unsupported),
        }
    }

    /// Releases everything the machine owns and reports how it ended.
    ///
    /// A forced release never asks the guest: the machine is ended and the receipt says so. A
    /// graceful one asks and reports whether the guest agreed, which is the only honest way to
    /// describe a termination a caller was told about.
    pub(super) fn release(
        self,
        instance: &InstanceId,
        forced: bool,
    ) -> Result<CleanupMethod, BackendFailureKind> {
        match self {
            Self::Resident(session) => Ok(release_resident(session, instance, forced)),
            Self::Jailed(jailed) => jailed.release(forced),
        }
    }
}

/// Releases a machine this process holds.
///
/// Dropping the session ends the sandbox thread, and the thread finishes the machine before it
/// returns, so a forced release needs nothing else.
fn release_resident(session: Session, instance: &InstanceId, forced: bool) -> CleanupMethod {
    if forced {
        drop(session);
        return CleanupMethod::Forced;
    }
    match session.shutdown() {
        Ok(evidence) => {
            dump_timeline(instance.as_str(), &evidence);
            shutdown_method(&evidence)
        }
        // The exchange did not complete, and the sandbox thread finished the machine on its way
        // out, so the machine is gone and the guest was not the one that ended it.
        Err(_) => CleanupMethod::GracefulThenForced,
    }
}

/// How a guest that was asked to shut down actually left.
///
/// A guest that halted, shut down, reset, or reached its sentinel left on its own, which is a
/// graceful release. Anything else means the host had to end a machine the guest was still in,
/// and reporting that as graceful would describe a termination that did not happen.
fn shutdown_method(evidence: &SandboxEvidence) -> CleanupMethod {
    match evidence.exit {
        Ok(GuestExit::Halt | GuestExit::Shutdown | GuestExit::Reset | GuestExit::Sentinel) => {
            CleanupMethod::Graceful
        }
        Ok(GuestExit::Paused) | Err(_) => CleanupMethod::GracefulThenForced,
    }
}
