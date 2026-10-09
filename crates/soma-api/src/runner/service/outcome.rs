//! What one failed request answers with.
//!
//! The mapping is the whole of the decision here, so it lives apart from the requests it
//! answers: a caller reads only the status and the code, and anything collapsed into a blanket
//! arm is information it can never recover. The rule the table encodes is that only a machine
//! which is genuinely gone is an agent outage.

use soma::{BackendFailureKind, ManagedFailure, ManagedStateError, RunFailureKind, TerminalStatus};

use crate::runner::public_wire::PlatformError;

use crate::runner::sandboxes::Unavailable;

/// The exit code one terminal status reports, when it is one a caller reads as a result.
///
/// A command a signal ended has no exit code of its own, so it takes the one every shell reports
/// for it: 128 above the signal. A backend that could not name the signal leaves the bare 128,
/// which still says the command was killed rather than that it exited. Every other status
/// describes a command that produced no result, and the failure carrying it says which.
pub(super) const fn exit_code(status: TerminalStatus) -> Option<i32> {
    match status {
        TerminalStatus::Exited { code } => Some(code),
        TerminalStatus::Signaled { signal } => Some(match signal {
            Some(signal) => 128 + signal,
            None => 128,
        }),
        TerminalStatus::TimedOut
        | TerminalStatus::OutputLimitExceeded
        | TerminalStatus::Failed
        | TerminalStatus::Ready
        | TerminalStatus::Stopped
        | TerminalStatus::Inspected { .. }
        | TerminalStatus::Destroyed => None,
    }
}

/// The platform error one failed command answers with.
///
/// Only a machine that is genuinely gone is an agent outage. A request the runner or the machine
/// refused is a refusal, and answering one as `AGENT_UNAVAILABLE` told the caller to retry a
/// request that could never succeed while the sandbox it was aimed at had already been released.
pub(super) fn failure_error(failure: &ManagedFailure) -> PlatformError {
    match failure {
        ManagedFailure::State(ManagedStateError::MachineNotFound) => {
            PlatformError::sandbox_not_found()
        }
        ManagedFailure::State(ManagedStateError::MachineStopped) => {
            PlatformError::sandbox_not_running()
        }
        ManagedFailure::Operation(failure) => run_failure_error(failure.kind()),
        ManagedFailure::Backend(kind) => backend_failure_error(*kind),
        _ => PlatformError::agent_unavailable(),
    }
}

/// The platform error one command outcome the engine named answers with.
///
/// Every outcome here came from the guest agent, which kills and reaps a command's process group
/// before it reports the command, or is the machine refusing the invocation outright. The engine
/// keeps the sandbox behind all of them, so each answer says so: none of these may send a caller
/// back to retry a sandbox that is no longer there, and none of them is the agent being gone.
/// A code names the outcome the caller can act on, and `sandbox_alive` states the fact the
/// caller would otherwise have to guess at after a failure with an exit code missing.
pub(super) fn run_failure_error(kind: RunFailureKind) -> PlatformError {
    match kind {
        RunFailureKind::TimedOut => alive(
            504,
            "EXEC_TIMEOUT",
            "the command was stopped at its deadline; the sandbox is still running",
            serde_json::json!(null),
        ),
        RunFailureKind::OutputLimitExceeded => alive(
            413,
            "EXEC_OUTPUT_LIMIT",
            "the command reached its output allowance and was stopped; the sandbox is still running",
            serde_json::json!(null),
        ),
        RunFailureKind::SpawnFailed { errno } => alive(
            400,
            "EXEC_SPAWN_FAILED",
            "the program could not be started, so no process ran; the sandbox is still running",
            serde_json::json!({"errno": errno}),
        ),
        RunFailureKind::Backend { kind, .. } => backend_failure_error(kind),
        _ => PlatformError::agent_unavailable(),
    }
}

/// One command outcome that the sandbox survived.
///
/// `details` is merged into the answer's own details so each refusal can name the field it is
/// about, which is what makes a refusal fixable rather than merely final.
fn alive(
    status: u16,
    code: &'static str,
    message: &str,
    details: serde_json::Value,
) -> PlatformError {
    let mut details = match details {
        serde_json::Value::Object(fields) => fields,
        _ => serde_json::Map::new(),
    };
    details.insert("sandbox_alive".to_owned(), serde_json::Value::Bool(true));
    PlatformError::new(status, code, message, false)
        .with_details(serde_json::Value::Object(details))
}

/// The platform error one backend refusal answers with.
///
/// These three are properties of the request rather than of the machine: a workload the sandbox
/// runtime will not take, a second operation against a sandbox already running one, and an
/// operation this backend does not serve. Each is answered by its own code so a caller can tell
/// them apart, and none of them is retryable because asking again changes none of them.
fn backend_failure_error(kind: BackendFailureKind) -> PlatformError {
    match kind {
        BackendFailureKind::WorkloadRejected => PlatformError::new(
            400,
            "WORKLOAD_REJECTED",
            "the sandbox runtime refused this request",
            false,
        ),
        BackendFailureKind::ResourceConflict => PlatformError::new(
            409,
            "RESOURCE_CONFLICT",
            "the sandbox is already running another operation",
            false,
        ),
        BackendFailureKind::Unsupported => PlatformError::new(
            501,
            "BACKEND_UNSUPPORTED",
            "the sandbox runtime does not serve this request",
            false,
        ),
        BackendFailureKind::Unavailable
        | BackendFailureKind::IsolationFailure
        | BackendFailureKind::GuestFailure
        | BackendFailureKind::Timeout
        | BackendFailureKind::OutputLimit
        | BackendFailureKind::CleanupFailure => PlatformError::agent_unavailable(),
    }
}

pub(super) fn command_refusal(unavailable: &Unavailable) -> PlatformError {
    match unavailable {
        Unavailable::NotFound => PlatformError::sandbox_not_found(),
        Unavailable::Busy => PlatformError::exec_busy(),
        Unavailable::Destroyed(..) => PlatformError::sandbox_not_running(),
    }
}
