//! Where a sandbox failure lands in the lifecycle contract.
//!
//! The sandbox reports the stage it stopped at; the contract reports which of the launch's
//! named points was not reached and what a caller should do about it. The mapping is written
//! out rather than collapsed to one failure because the point a caller reads has to be the
//! point that actually failed.

use soma_guest::TerminalStatus;

use crate::platform::{ReadinessFailure, ReadinessProgress, ReadinessStep};
use crate::sandbox::SessionError;
use crate::{ExitStatus, Recovery};

/// What a caller should do about a machine that could not be restored.
pub(super) const fn restore_recovery(error: SessionError) -> Recovery {
    match error {
        // The artifacts, the host profile, or the snapshot itself did not produce a machine.
        // That is a property of this host and its store rather than of the request.
        SessionError::Create | SessionError::LaunchPage | SessionError::Network => {
            Recovery::RepairHost
        }
        // A machine existed and then stopped being usable, so another one may well work.
        _ => Recovery::ReplaceMachine,
    }
}

/// What a caller should do about a command that produced no certain answer.
pub(super) const fn execute_recovery(error: SessionError) -> Recovery {
    match error {
        // Every one of these ends the session, and the sandbox thread releases the machine on
        // its way out, so the Instance is finished and no retry of the command can change that.
        SessionError::Poisoned
        | SessionError::Gone
        | SessionError::Execute
        | SessionError::File
        | SessionError::Pty
        | SessionError::Workspace
        | SessionError::Pause
        | SessionError::Resume => Recovery::ReplaceMachine,
        // Nothing was sent and the session stands, but this guest refuses the same command again.
        SessionError::GuestAgentOutdated | SessionError::GuestPaused => Recovery::DoNotRetry,
        _ => Recovery::RepairHost,
    }
}

/// The readiness point one sandbox failure corresponds to.
///
/// The sandbox reaches its guest through one sequence: it publishes the launch material, the
/// guest authenticates and acknowledges the Generation, repair fixes identity and network, and
/// only then does the readiness receipt exist. A failure before the session exists therefore
/// completed none of those points, a failure placing a secret completed everything up to and
/// including repair, and a failure of the receipt itself completed all four.
pub(super) fn readiness(error: SessionError) -> ReadinessFailure {
    match error {
        SessionError::Create
        | SessionError::CheckpointIncompatible
        | SessionError::LaunchPage
        | SessionError::Boot => {
            ReadinessFailure::for_platform(ReadinessProgress::from_steps([]), Recovery::RepairHost)
        }
        SessionError::Secret | SessionError::Network => ReadinessFailure::for_platform(
            ReadinessProgress::from_steps([
                ReadinessStep::GuestAuthenticated,
                ReadinessStep::GenerationAcknowledged,
                ReadinessStep::IdentityRepaired,
            ]),
            Recovery::ReplaceMachine,
        ),
        SessionError::Ready
        | SessionError::Execute
        | SessionError::GuestAgentOutdated
        | SessionError::Pause
        | SessionError::Resume
        | SessionError::GuestPaused
        | SessionError::File
        | SessionError::Pty
        | SessionError::Workspace
        | SessionError::Gone
        | SessionError::Poisoned
        | SessionError::Suspend => ReadinessFailure::for_platform(
            ReadinessProgress::from_steps([
                ReadinessStep::GuestAuthenticated,
                ReadinessStep::GenerationAcknowledged,
                ReadinessStep::IdentityRepaired,
                ReadinessStep::NetworkRepaired,
            ]),
            Recovery::ReplaceMachine,
        ),
    }
}

/// The contract status one guest terminal status is.
///
/// A command the guest could not start produced no process, so it has no exit code, but the
/// agent that answered is alive and had nothing to run. That is a refusal with a name, and it
/// travels as one. A command the agent itself could not carry out says the agent is not
/// trustworthy, which no status describes, so it returns nothing and becomes a typed failure.
pub(super) const fn exit_status(status: TerminalStatus) -> Option<ExitStatus> {
    match status {
        TerminalStatus::Exited(code) => Some(ExitStatus::Code(code)),
        TerminalStatus::Signaled(signal) => Some(ExitStatus::Signal(signal)),
        TerminalStatus::TimedOut => Some(ExitStatus::TimedOut),
        TerminalStatus::OutputLimit => Some(ExitStatus::OutputLimit),
        TerminalStatus::ExecFailed(errno) => Some(ExitStatus::SpawnFailed(errno)),
        TerminalStatus::AgentFailed(_) => None,
    }
}

/// Whether the guest ended the machine itself rather than being ended inside it.
pub(super) fn left_on_its_own<E>(exit: &Result<soma_kvm::x86_64::GuestExit, E>) -> bool {
    use soma_kvm::x86_64::GuestExit;
    matches!(
        exit,
        Ok(GuestExit::Halt | GuestExit::Shutdown | GuestExit::Reset | GuestExit::Sentinel)
    )
}

/// The device's own answer, inside the platform's account of whether the session survived.
pub(super) type MemoryAnswer = Result<
    Result<crate::sandbox::MemoryLevel, crate::sandbox::MemoryError>,
    super::PlatformFailure,
>;

/// One exchange with the running machine's memory device.
pub(super) fn memory(
    session: Option<&mut crate::sandbox::Session>,
    requested: Option<u64>,
) -> MemoryAnswer {
    let session = session.ok_or_else(|| super::PlatformFailure::new(Recovery::RepairHost))?;
    match requested {
        Some(bytes) => session.set_requested_memory(bytes),
        None => session.memory(),
    }
    .map_err(|error| super::PlatformFailure::new(execute_recovery(error)))
}
