//! Reading one worker reply as the thing the broker asked for, and nothing else.
//!
//! Each helper admits exactly the outcome its request has, so a worker that answers a Launch
//! with an Executed receipt is a protocol failure rather than a launch that half succeeded.

use soma::BackendFailureKind;
use soma_guest::TerminalStatus;
use soma_jail::{ProbeReport, RootView};
use soma_vmm::control::{Outcome, OutputStream};
use soma_vmm::{ExitStatus, FailureKind};

/// The report a handle holds before its worker has attested anything.
pub(super) const UNATTESTED: ProbeReport = ProbeReport {
    pid: 0,
    uid: 0,
    euid: 0,
    gid: 0,
    egid: 0,
    table_sealed: false,
    first_bad_slot: None,
    root: RootView {
        entries: 0,
        writable: true,
        proc_visible: true,
        sys_visible: true,
    },
};

/// Whether an attestation proves the worker is inside the jail this broker built.
///
/// The properties are the jail's own guarantees: a sealed descriptor table with no unexpected
/// slot, an unprivileged identity that is PID 1 of its own namespace, and an empty read-only
/// root with neither procfs nor sysfs in it. A report failing any of them describes a process
/// somewhere else, and the machine it holds is not contained.
pub(super) fn describes_a_jail(report: &ProbeReport) -> bool {
    let identities = [report.uid, report.euid, report.gid, report.egid];
    report.table_sealed
        && report.first_bad_slot.is_none()
        && report.pid == 1
        && identities.iter().all(|identity| *identity != 0)
        && report.root.entries == 0
        && !report.root.writable
        && !report.root.proc_visible
        && !report.root.sys_visible
}

/// Admits only a Launch that reached authenticated command readiness.
pub(super) fn ready(outcome: &Outcome) -> Result<(), BackendFailureKind> {
    match outcome {
        Outcome::Ready { .. } => Ok(()),
        Outcome::Failure { kind, .. } => Err(refusal(*kind)),
        _ => Err(BackendFailureKind::Unavailable),
    }
}

/// Admits only the acknowledgement that the filter reached its steady state.
pub(super) fn sealed(outcome: &Outcome) -> Result<(), BackendFailureKind> {
    match outcome {
        Outcome::Sealed => Ok(()),
        // A worker that cannot narrow its own filter is holding a machine under a wider policy
        // than this host admits, so the launch fails rather than serving it anyway.
        _ => Err(BackendFailureKind::IsolationFailure),
    }
}

/// Admits only a completed command, with the byte counts its output has.
pub(super) fn executed(
    outcome: &Outcome,
) -> Result<(TerminalStatus, u64, u64), BackendFailureKind> {
    match outcome {
        Outcome::Executed {
            status,
            stdout_bytes,
            stderr_bytes,
            ..
        } => Ok((terminal(*status), *stdout_bytes, *stderr_bytes)),
        Outcome::Failure { kind, .. } => Err(refusal(*kind)),
        _ => Err(BackendFailureKind::Unavailable),
    }
}

/// Admits only one window of output.
pub(super) fn output(outcome: Outcome) -> Result<Vec<u8>, BackendFailureKind> {
    match outcome {
        Outcome::Output(bytes) => Ok(bytes),
        _ => Err(BackendFailureKind::Unavailable),
    }
}

/// Admits only a stop that proved its cleanup, and reports whether the guest agreed to it.
pub(super) fn stopped(outcome: &Outcome) -> Option<bool> {
    match outcome {
        Outcome::Stopped {
            guest_acknowledged, ..
        } => Some(*guest_acknowledged),
        _ => None,
    }
}

/// The status a command's guest terminal status is on this side of the split.
const fn terminal(status: ExitStatus) -> TerminalStatus {
    match status {
        ExitStatus::Code(code) => TerminalStatus::Exited(code),
        ExitStatus::Signal(signal) => TerminalStatus::Signaled(signal),
        ExitStatus::TimedOut => TerminalStatus::TimedOut,
        ExitStatus::OutputLimit => TerminalStatus::OutputLimit,
    }
}

/// The Backend refusal one contract failure is.
const fn refusal(kind: FailureKind) -> BackendFailureKind {
    match kind {
        // The request was not one the Machine could be asked for in the state it was in.
        FailureKind::OperationConflict
        | FailureKind::OperationCapacityExceeded
        | FailureKind::InvalidLifecycle
        | FailureKind::InstanceMismatch => BackendFailureKind::ResourceConflict,
        // The artifacts this host presented as prepared did not produce a machine.
        FailureKind::GenerationVerificationFailed | FailureKind::RestoreFailed => {
            BackendFailureKind::Unavailable
        }
        // A machine existed and its guest never reached, or lost, its authenticated session.
        FailureKind::GuestAuthenticationFailed
        | FailureKind::GenerationAcknowledgementFailed
        | FailureKind::IdentityRepairFailed
        | FailureKind::NetworkRepairFailed
        | FailureKind::ReadinessProbeFailed
        | FailureKind::ExecuteFailed => BackendFailureKind::GuestFailure,
        FailureKind::StopFailed => BackendFailureKind::CleanupFailure,
    }
}

/// The stream a window reads, named here so the caller does not import the control module twice.
pub(super) const STDOUT: OutputStream = OutputStream::Stdout;
/// The other stream a window reads.
pub(super) const STDERR: OutputStream = OutputStream::Stderr;
