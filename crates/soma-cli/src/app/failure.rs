use soma::{
    BackendFailureKind, FailurePhase, ManagedFailure, ManagedStateError, RunFailure, RunFailureKind,
};
use soma_local::LocalFailureKind;

use crate::{
    exit::ProcessExit,
    model::{CommandReport, FailureBody, OutputBytes, Response, ResultBody},
    request::RequestError,
};

use super::{Execution, success::command_status};

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
        ManagedFailure::StateStore(_) => software_failure(command, "state_store_failure"),
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

pub(super) fn local_failure(command: &'static str, kind: LocalFailureKind) -> Execution {
    let (body, exit) = match kind {
        LocalFailureKind::InvalidConfiguration => (
            FailureBody::new(
                "invalid_configuration",
                "local runtime configuration is invalid",
                false,
            ),
            ProcessExit::InvalidInput,
        ),
        LocalFailureKind::UnsupportedTarget => (
            FailureBody::new(
                "unsupported_backend",
                "local backend is unsupported on this host",
                false,
            ),
            ProcessExit::UnsupportedBackend,
        ),
        LocalFailureKind::BackendUnavailable => (
            FailureBody::new(
                "backend_unavailable",
                "local isolation backend is unavailable",
                true,
            ),
            ProcessExit::CapabilityUnavailable,
        ),
        LocalFailureKind::StateStore(_) => (
            FailureBody::new(
                "state_store_failure",
                "durable state store could not be opened",
                true,
            ),
            ProcessExit::Software,
        ),
    };
    Execution {
        response: Response::failure(command, body),
        exit,
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

fn failure_details(kind: RunFailureKind) -> (FailureBody, ProcessExit) {
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
        RunFailureKind::StateStore { .. } => (
            FailureBody::new(
                "state_store_failure",
                "durable state operation failed",
                true,
            ),
            ProcessExit::Software,
        ),
        RunFailureKind::Backend { phase, kind } => backend_failure_details(phase, kind),
    }
}

fn backend_failure_details(
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
                true,
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
