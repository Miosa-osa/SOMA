use crate::{CleanupEvidence, Failure, FailureKind, FailurePhase, Milestones, Recovery};

pub(super) fn operation_conflict() -> Failure {
    Failure::new(
        FailureKind::OperationConflict,
        FailurePhase::Idempotency,
        Recovery::DoNotRetry,
        Milestones::default(),
        CleanupEvidence::NotRequired,
    )
}

pub(super) fn operation_capacity_exceeded() -> Failure {
    Failure::new(
        FailureKind::OperationCapacityExceeded,
        FailurePhase::Idempotency,
        Recovery::DoNotRetry,
        Milestones::default(),
        CleanupEvidence::NotRequired,
    )
}

pub(super) fn lifecycle_failure() -> Failure {
    Failure::new(
        FailureKind::InvalidLifecycle,
        FailurePhase::Lifecycle,
        Recovery::DoNotRetry,
        Milestones::default(),
        CleanupEvidence::NotRequired,
    )
}

pub(super) fn instance_mismatch() -> Failure {
    Failure::new(
        FailureKind::InstanceMismatch,
        FailurePhase::Lifecycle,
        Recovery::DoNotRetry,
        Milestones::default(),
        CleanupEvidence::NotRequired,
    )
}
