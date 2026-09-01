use soma::{
    BackendFailureKind, FailurePhase, ManagedFailure, ManagedStateError, RunFailure,
    RunFailureKind, StateStoreFailureKind,
};

use crate::{
    exit::ProcessExit,
    model::{CommandReport, FailureBody, OutputBytes, Response, ResultBody},
    request::RequestError,
};

use super::{Execution, success::command_status};

#[path = "failure/local.rs"]
mod local;
pub(super) use local::{local_failure, not_hosted};

pub(super) fn run_failure(
    command: &'static str,
    instance_id: soma::InstanceId,
    failure: &RunFailure,
) -> Execution {
    let (body, exit) = failure_details(failure.kind());
    let result = failure.output().and_then(|output| {
        command_status(*failure.receipt().terminal_status()).map(|status| {
            ResultBody::Command(CommandReport {
                instance_id,
                execution: status,
                stdout: OutputBytes::new(output.stdout()),
                stderr: OutputBytes::new(output.stderr()),
            })
        })
    });
    Execution {
        response: Response::failure_with_receipt(command, result, body, failure.receipt().clone()),
        exit,
    }
}

pub(super) fn managed_failure(
    command: &'static str,
    instance_id: soma::InstanceId,
    failure: &ManagedFailure,
) -> Execution {
    match failure {
        ManagedFailure::Operation(failure) => run_failure(command, instance_id, failure),
        ManagedFailure::State(state) => {
            let (body, exit) = managed_state_details(*state);
            Execution {
                response: Response::failure(command, body),
                exit,
            }
        }
        ManagedFailure::StateStore(kind) => Execution {
            response: Response::failure(command, state_store_body(*kind)),
            exit: ProcessExit::Software,
        },
        // An operation that mints no receipt reports the backend kind directly. The phase named
        // here is only for the shared table's benefit; it does not change any code or status.
        ManagedFailure::Backend(kind) => {
            let (body, exit) = backend_failure_details(FailurePhase::Command, *kind);
            Execution {
                response: Response::failure(command, body),
                exit,
            }
        }
        ManagedFailure::ReplayUnavailable(replay) => {
            let body = FailureBody::new(
                "replay_unavailable",
                "the operation completed but its full replay payload is unavailable",
                false,
            );
            replay.receipt().map_or_else(
                || Execution {
                    response: Response::failure(command, body),
                    exit: ProcessExit::Conflict,
                },
                |receipt| Execution {
                    response: Response::failure_with_receipt(command, None, body, receipt.clone()),
                    exit: ProcessExit::Conflict,
                },
            )
        }
    }
}

/// Reports a failure from an operation that names no Instance.
///
/// An enumeration is the only one. It reads the store and asks the backend what it can reach, so
/// the failures it can produce are the store's; an operation failure carries a receipt for one
/// Instance, which an enumeration never asked about, and reaching this with one would mean the
/// engine answered about a sandbox that was not part of the question.
pub(super) fn listing_failure(command: &'static str, failure: &ManagedFailure) -> Execution {
    match failure {
        ManagedFailure::StateStore(kind) => Execution {
            response: Response::failure(command, state_store_body(*kind)),
            exit: ProcessExit::Software,
        },
        ManagedFailure::State(state) => {
            let (body, exit) = managed_state_details(*state);
            Execution {
                response: Response::failure(command, body),
                exit,
            }
        }
        ManagedFailure::Backend(_)
        | ManagedFailure::Operation(_)
        | ManagedFailure::ReplayUnavailable(_) => Execution {
            response: Response::failure(
                command,
                FailureBody::new(
                    "internal_error",
                    "the listing reported a failure that names one sandbox",
                    false,
                ),
            ),
            exit: ProcessExit::Software,
        },
    }
}

pub(super) fn invalid(command: &'static str, error: RequestError) -> Execution {
    Execution {
        response: Response::failure(command, FailureBody::invalid(error.reason())),
        exit: ProcessExit::InvalidInput,
    }
}

pub(super) fn software_failure(command: &'static str, message: &'static str) -> Execution {
    Execution {
        response: Response::failure(
            command,
            FailureBody::new("internal_contract_failure", message, false),
        ),
        exit: ProcessExit::Software,
    }
}

pub(super) fn failure_details(kind: RunFailureKind) -> (FailureBody, ProcessExit) {
    match kind {
        RunFailureKind::TimedOut => (
            FailureBody::new("guest_timeout", "guest command exceeded its deadline", true),
            ProcessExit::GuestTimeout,
        ),
        RunFailureKind::OutputLimitExceeded => (
            FailureBody::new(
                "output_limit",
                "guest output exceeded its declared allowance",
                false,
            ),
            ProcessExit::OutputLimit,
        ),
        RunFailureKind::Interrupted => (
            FailureBody::new("guest_interrupted", "guest execution was interrupted", true),
            ProcessExit::GuestNonzero,
        ),
        RunFailureKind::CleanupIncomplete => (
            FailureBody::new(
                "cleanup_incomplete",
                "sandbox cleanup could not be proven",
                false,
            ),
            ProcessExit::CleanupUncertain,
        ),
        RunFailureKind::ObservationMismatch => (
            FailureBody::new(
                "evidence_mismatch",
                "backend evidence violated the SOMA contract",
                false,
            ),
            ProcessExit::Software,
        ),
        RunFailureKind::StateStore { kind } => (state_store_body(kind), ProcessExit::Software),
        RunFailureKind::Backend { phase, kind } => backend_failure_details(phase, kind),
    }
}

pub(super) fn backend_failure_details(
    _phase: FailurePhase,
    kind: BackendFailureKind,
) -> (FailureBody, ProcessExit) {
    match kind {
        BackendFailureKind::Unsupported => (
            FailureBody::new(
                "unsupported_backend",
                "backend operation is unsupported",
                false,
            ),
            ProcessExit::UnsupportedBackend,
        ),
        BackendFailureKind::Unavailable => (
            FailureBody::new(
                "backend_unavailable",
                "backend capability is unavailable",
                false,
            ),
            ProcessExit::CapabilityUnavailable,
        ),
        BackendFailureKind::ResourceConflict => (
            FailureBody::new(
                "resource_conflict",
                "requested host resource is already in use",
                false,
            ),
            ProcessExit::Conflict,
        ),
        BackendFailureKind::WorkloadRejected => (
            FailureBody::new("workload_rejected", "workload was rejected", false),
            ProcessExit::InvalidInput,
        ),
        BackendFailureKind::Timeout => failure_details(RunFailureKind::TimedOut),
        BackendFailureKind::OutputLimit => failure_details(RunFailureKind::OutputLimitExceeded),
        BackendFailureKind::CleanupFailure => failure_details(RunFailureKind::CleanupIncomplete),
        BackendFailureKind::IsolationFailure | BackendFailureKind::GuestFailure => (
            FailureBody::new("backend_failure", "sandbox backend operation failed", true),
            ProcessExit::BackendFailure,
        ),
    }
}

fn managed_state_details(kind: ManagedStateError) -> (FailureBody, ProcessExit) {
    match kind {
        ManagedStateError::MachineNotFound => (
            FailureBody::new("machine_not_found", "sandbox instance was not found", false),
            ProcessExit::NotFound,
        ),
        ManagedStateError::MachineAlreadyExists
        | ManagedStateError::MachineStopped
        | ManagedStateError::OperationConflict
        | ManagedStateError::RecoveryRequired
        | ManagedStateError::ReplayCapacityReached => (
            FailureBody::new(
                "state_conflict",
                "sandbox state rejected the operation",
                false,
            ),
            ProcessExit::Conflict,
        ),
    }
}

/// The failure body one durable state store condition reports.
fn state_store_body(kind: StateStoreFailureKind) -> FailureBody {
    FailureBody::new(
        "state_store_failure",
        "durable state operation failed",
        state_store_retryable(kind),
    )
}

/// Whether resubmitting the identical request, with no operator action in between, could
/// succeed.
///
/// A lock another caller holds and a store that is momentarily unreachable are conditions that
/// clear on their own. A record that is corrupt, malformed, written by a version this build does
/// not understand, or already at capacity does not: retrying those is a loop that cannot end,
/// which is exactly what `retryable` is read as promising will not happen.
pub(super) const fn state_store_retryable(kind: StateStoreFailureKind) -> bool {
    matches!(
        kind,
        StateStoreFailureKind::Conflict | StateStoreFailureKind::Unavailable
    )
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
