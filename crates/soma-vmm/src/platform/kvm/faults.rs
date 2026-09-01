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
        SessionError::Poisoned | SessionError::Gone | SessionError::Execute => {
            Recovery::ReplaceMachine
        }
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
        SessionError::Create | SessionError::LaunchPage | SessionError::Boot => {
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
        | SessionError::Gone
        | SessionError::Poisoned => ReadinessFailure::for_platform(
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
/// A command the guest could not start, or that its own agent failed to run, produced no
/// process and therefore no status a caller may read as one. Those are refusals of the
/// execution rather than results of it, so they return nothing and become a typed failure.
pub(super) const fn exit_status(status: TerminalStatus) -> Option<ExitStatus> {
    match status {
        TerminalStatus::Exited(code) => Some(ExitStatus::Code(code)),
        TerminalStatus::Signaled(signal) => Some(ExitStatus::Signal(signal)),
        TerminalStatus::TimedOut => Some(ExitStatus::TimedOut),
        TerminalStatus::OutputLimit => Some(ExitStatus::OutputLimit),
        TerminalStatus::ExecFailed(_) | TerminalStatus::AgentFailed(_) => None,
    }
}
