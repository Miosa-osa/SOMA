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
    let error = refusal(failure);
    if let Some(line) = server_error_line(failure, error) {
        eprintln!("{line}");
    }
    error
}

/// The log line a refusal the server is responsible for leaves, naming its cause; none for a
/// refusal of the caller's request. A 5xx with nothing on the service's log could not be
/// diagnosed after the fact.
#[must_use]
pub fn server_error_line(failure: &ManagedFailure, error: ApiError) -> Option<String> {
    if error.status() < 500 {
        return None;
    }
    let cause = match failure {
        ManagedFailure::Operation(run) => format!(
            "{:?} on {:?} {}",
            run.kind(),
            run.receipt().operation(),
            run.receipt().instance_id().as_str()
        ),
        ManagedFailure::State(state) => format!("state {state:?}"),
        ManagedFailure::StateStore(kind) => format!("state store {kind:?}"),
        ManagedFailure::Backend(kind) => format!("backend {kind:?}"),
        ManagedFailure::ReplayUnavailable(_) => "replay unavailable".to_owned(),
    };
    Some(format!(
        "soma-api: answered {} {}: {cause}",
        error.status(),
        error.body().code
    ))
}

fn refusal(failure: &ManagedFailure) -> ApiError {
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
        RunFailureKind::SpawnFailed { .. } => ApiError::new(
            400,
            "guest_spawn_failed",
            "guest could not start the program, so no process ran",
            false,
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
        BackendFailureKind::CheckpointIncompatible => ApiError::new(
            409,
            "checkpoint_incompatible",
            "this checkpoint requires a compatible host",
            false,
        ),
        BackendFailureKind::SandboxPaused => {
            ApiError::new(409, "sandbox_paused", "the sandbox is paused", false)
        }
        BackendFailureKind::ResourceConflict => ApiError::new(
            409,
            "resource_conflict",
            "requested host resource is already in use",
            false,
        ),
        BackendFailureKind::WorkloadRejected => {
            ApiError::new(400, "workload_rejected", "workload was rejected", false)
        }
        // Retryable, and not a 400: the request is sound, the host is full now, and a caller
        // with other hosts places it there.
        BackendFailureKind::CapacityExhausted => ApiError::new(
            503,
            "capacity_exhausted",
            "this host has no room for the sandbox now",
            true,
        ),
        BackendFailureKind::GuestAgentOutdated => ApiError::new(
            422,
            "guest_agent_outdated",
            "the sandbox's guest agent carries arguments of at most 4096 bytes; capture its \
             Generation again with a newer agent to run this command",
            false,
        ),
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

#[cfg(test)]
mod logging_tests {
    use super::*;

    #[test]
    fn a_server_failure_names_its_cause_on_the_log() {
        let failure = ManagedFailure::StateStore(StateStoreFailureKind::Corrupt);
        let line = server_error_line(&failure, managed_error(&failure)).expect("a 5xx is logged");
        assert!(line.contains("500 state_store_failure"), "{line}");
        assert!(line.contains("state store Corrupt"), "{line}");
    }

    /// A full host is retryable elsewhere, and a request no host would run stays a 400.
    #[test]
    fn a_full_host_is_a_retryable_capacity_refusal() {
        let full = managed_error(&ManagedFailure::Backend(
            soma::BackendFailureKind::CapacityExhausted,
        ));
        assert_eq!(
            (full.status(), full.body().code, full.body().retryable),
            (503, "capacity_exhausted", true)
        );
        let rejected = managed_error(&ManagedFailure::Backend(
            soma::BackendFailureKind::WorkloadRejected,
        ));
        assert_eq!(
            (rejected.status(), rejected.body().code),
            (400, "workload_rejected")
        );
    }

    #[test]
    fn a_refusal_of_the_callers_request_is_not_logged() {
        let failure = ManagedFailure::State(ManagedStateError::MachineNotFound);
        assert_eq!(server_error_line(&failure, managed_error(&failure)), None);
    }
}
