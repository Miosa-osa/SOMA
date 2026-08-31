//! How the facade's typed failures become the ones this server reports.
//!
//! They live apart from the tool bodies because the mapping is the whole of the decision: a
//! caller reads only the kind, so anything collapsed here is information it can never recover.

use soma::{BackendFailureKind, ManagedFailure, ManagedStateError, RunFailure, RunFailureKind};

use crate::{RuntimeFailure, RuntimeFailureKind};

use super::{receipt, terminal_status};

pub(super) fn map_run_failure(failure: &RunFailure) -> RuntimeFailure {
    let kind = map_run_failure_kind(failure.kind());
    let Ok(receipt) = receipt(failure.receipt()) else {
        return RuntimeFailure::new(RuntimeFailureKind::Internal);
    };
    if let (Some(output), Some(status)) = (
        failure.output(),
        terminal_status(*failure.receipt().terminal_status()),
    ) {
        RuntimeFailure::with_command_evidence(
            kind,
            receipt,
            status,
            output.stdout().to_vec(),
            output.stderr().to_vec(),
        )
    } else {
        RuntimeFailure::with_receipt(kind, receipt)
    }
}

pub(super) fn map_managed_failure(failure: &ManagedFailure) -> RuntimeFailure {
    match failure {
        ManagedFailure::Operation(failure) => map_run_failure(failure),
        ManagedFailure::State(state) => RuntimeFailure::new(match state {
            ManagedStateError::MachineNotFound => RuntimeFailureKind::NotFound,
            ManagedStateError::MachineAlreadyExists
            | ManagedStateError::MachineStopped
            | ManagedStateError::OperationConflict
            | ManagedStateError::RecoveryRequired
            | ManagedStateError::ReplayCapacityReached => RuntimeFailureKind::Conflict,
        }),
        ManagedFailure::StateStore(_) => RuntimeFailure::new(RuntimeFailureKind::Internal),
        ManagedFailure::ReplayUnavailable(replay) => replay.receipt().map_or_else(
            || RuntimeFailure::new(RuntimeFailureKind::Conflict),
            |evidence| {
                receipt(evidence).map_or_else(
                    |_| RuntimeFailure::new(RuntimeFailureKind::Internal),
                    |receipt| RuntimeFailure::with_receipt(RuntimeFailureKind::Conflict, receipt),
                )
            },
        ),
    }
}

const fn map_run_failure_kind(kind: RunFailureKind) -> RuntimeFailureKind {
    match kind {
        RunFailureKind::Backend { kind, .. } => match kind {
            BackendFailureKind::Unsupported => RuntimeFailureKind::Unsupported,
            BackendFailureKind::Unavailable => RuntimeFailureKind::Unavailable,
            BackendFailureKind::ResourceConflict => RuntimeFailureKind::Conflict,
            BackendFailureKind::WorkloadRejected => RuntimeFailureKind::Rejected,
            BackendFailureKind::Timeout => RuntimeFailureKind::Timeout,
            BackendFailureKind::OutputLimit => RuntimeFailureKind::OutputLimit,
            BackendFailureKind::CleanupFailure => RuntimeFailureKind::CleanupIncomplete,
            BackendFailureKind::IsolationFailure | BackendFailureKind::GuestFailure => {
                RuntimeFailureKind::Internal
            }
        },
        RunFailureKind::TimedOut => RuntimeFailureKind::Timeout,
        RunFailureKind::OutputLimitExceeded => RuntimeFailureKind::OutputLimit,
        RunFailureKind::CleanupIncomplete => RuntimeFailureKind::CleanupIncomplete,
        RunFailureKind::Interrupted
        | RunFailureKind::ObservationMismatch
        | RunFailureKind::StateStore { .. } => RuntimeFailureKind::Internal,
    }
}
