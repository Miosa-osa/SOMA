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

use std::sync::OnceLock;
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
            Self::Jailed(jailed) => jailed.pty(operation),
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

/// The switch that makes a forced release end the machine without asking the guest.
///
/// Off by default. Without it, dropping the session closes its request channel and the sandbox
/// thread treats that as an ordinary end: it asks the guest to shut down and waits for the guest
/// kernel's whole poweroff path, so a "forced" destroy costs a graceful one.
const IMMEDIATE_FORCED_RELEASE: &str = "SOMA_IMMEDIATE_FORCED_DESTROY";

/// Whether a forced release aborts the machine rather than shutting the guest down.
///
/// Read once per process. A machine host inherits the environment of the service that spawned
/// it, so the service's setting is the one every machine it holds follows.
pub(super) fn immediate_forced_release() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| enabled(std::env::var(IMMEDIATE_FORCED_RELEASE).ok().as_deref()))
}

/// Only an explicit `1` or `true` turns a switch on; anything else leaves the default.
pub(super) fn enabled(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true"))
}

/// Releases a machine this process holds.
///
/// A forced release ends the sandbox thread, and the thread finishes the machine before it
/// returns, so it needs nothing else. With [`IMMEDIATE_FORCED_RELEASE`] on, the thread is told
/// to abort, which kicks the vCPU out of `KVM_RUN` without asking the guest; otherwise dropping
/// the session asks the guest to shut down first.
fn release_resident(session: Session, instance: &InstanceId, forced: bool) -> CleanupMethod {
    if forced {
        if immediate_forced_release() {
            // Either answer means the same thing here: the thread has ended and finished the
            // machine on its way out, so nothing of it is left running.
            let _ignored = session.abort();
        } else {
            drop(session);
        }
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

#[cfg(test)]
mod tests {
    use super::enabled;

    #[test]
    fn the_immediate_release_switch_is_off_unless_explicitly_on() {
        assert!(!enabled(None));
        for off in ["", "0", "false", "yes", "TRUE", " 1"] {
            assert!(!enabled(Some(off)), "{off:?} must leave the default");
        }
        assert!(enabled(Some("1")));
        assert!(enabled(Some("true")));
    }
}
