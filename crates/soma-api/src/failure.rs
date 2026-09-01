use soma::{
    BackendFailureKind, ManagedFailure, ManagedStateError, RunFailure, RunFailureKind,
    StateStoreFailureKind,
};

use crate::envelope::ApiError;

/// Translates a facade managed failure into an HTTP refusal.
///
/// Every code and message here is copied from the CLI's failure table rather than reworded, so
/// the same engine condition reports identically whether an operator reached it through the CLI
/// or through this service. Only the HTTP status is new, because the CLI carries a process exit
/// code in its place.
#[must_use]
pub fn managed_error(failure: &ManagedFailure) -> ApiError {
    match failure {
        ManagedFailure::Operation(failure) => run_error(failure),
        ManagedFailure::State(state) => state_error(*state),
        ManagedFailure::StateStore(kind) => state_store_error(*kind),
        // An operation that mints no receipt reports the backend kind directly; the mapping is
        // the same one a receipt-carrying backend failure goes through.
        ManagedFailure::Backend(kind) => backend_error(*kind),
        ManagedFailure::ReplayUnavailable(_) => ApiError::new(
            409,
            "replay_unavailable",
            "the operation completed but its full replay payload is unavailable",
            false,
        ),
    }
}

#[must_use]
pub fn run_error(failure: &RunFailure) -> ApiError {
    run_kind_error(failure.kind())
}

fn run_kind_error(kind: RunFailureKind) -> ApiError {
    match kind {
        RunFailureKind::TimedOut => ApiError::new(
            504,
            "guest_timeout",
            "guest command exceeded its deadline",
            true,
        ),
        RunFailureKind::OutputLimitExceeded => ApiError::new(
            413,
            "output_limit",
            "guest output exceeded its declared allowance",
            false,
        ),
        RunFailureKind::Interrupted => ApiError::new(
            502,
            "guest_interrupted",
            "guest execution was interrupted",
            true,
        ),
        RunFailureKind::CleanupIncomplete => ApiError::new(
            500,
            "cleanup_incomplete",
            "sandbox cleanup could not be proven",
            false,
        ),
        RunFailureKind::ObservationMismatch => ApiError::new(
            500,
            "evidence_mismatch",
            "backend evidence violated the SOMA contract",
            false,
        ),
        RunFailureKind::StateStore { kind } => state_store_error(kind),
        RunFailureKind::Backend { kind, .. } => backend_error(kind),
    }
}

fn backend_error(kind: BackendFailureKind) -> ApiError {
    match kind {
        BackendFailureKind::Unsupported => ApiError::new(
            501,
            "unsupported_backend",
            "backend operation is unsupported",
            false,
        ),
        // Not retryable: a capability this host does not have does not appear because a caller
        // asked again. Clearing it takes operator action, and a client that reads `retryable`
        // as permission to keep asking would loop forever.
        BackendFailureKind::Unavailable => ApiError::new(
            503,
            "backend_unavailable",
            "backend capability is unavailable",
            false,
        ),
        BackendFailureKind::ResourceConflict => ApiError::new(
            409,
            "resource_conflict",
            "requested host resource is already in use",
            false,
        ),
        BackendFailureKind::WorkloadRejected => {
            ApiError::new(400, "workload_rejected", "workload was rejected", false)
        }
        BackendFailureKind::Timeout => run_kind_error(RunFailureKind::TimedOut),
        BackendFailureKind::OutputLimit => run_kind_error(RunFailureKind::OutputLimitExceeded),
        BackendFailureKind::CleanupFailure => run_kind_error(RunFailureKind::CleanupIncomplete),
        BackendFailureKind::IsolationFailure | BackendFailureKind::GuestFailure => ApiError::new(
            502,
            "backend_failure",
            "sandbox backend operation failed",
            true,
        ),
    }
}

/// The refusal one durable state store condition reports.
///
/// Only a lock another caller holds and a momentarily unreachable store clear on their own. A
/// corrupt, malformed, unknown-version, or full record does not, and marking those retryable
/// invites a loop that cannot end.
fn state_store_error(kind: StateStoreFailureKind) -> ApiError {
    ApiError::new(
        500,
        "state_store_failure",
        "durable state operation failed",
        matches!(
            kind,
            StateStoreFailureKind::Conflict | StateStoreFailureKind::Unavailable
        ),
    )
}

fn state_error(kind: ManagedStateError) -> ApiError {
    match kind {
        ManagedStateError::MachineNotFound => ApiError::new(
            404,
            "machine_not_found",
            "sandbox instance was not found",
            false,
        ),
        ManagedStateError::MachineAlreadyExists
        | ManagedStateError::MachineStopped
        | ManagedStateError::OperationConflict
        | ManagedStateError::RecoveryRequired
        | ManagedStateError::ReplayCapacityReached => ApiError::new(
            409,
            "state_conflict",
            "sandbox state rejected the operation",
            false,
        ),
    }
}
