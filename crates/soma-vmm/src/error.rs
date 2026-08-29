use std::{error::Error, fmt};

use crate::Milestones;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    OperationConflict,
    OperationCapacityExceeded,
    InvalidLifecycle,
    GenerationVerificationFailed,
    RestoreFailed,
    GuestAuthenticationFailed,
    GenerationAcknowledgementFailed,
    IdentityRepairFailed,
    NetworkRepairFailed,
    ReadinessProbeFailed,
    InstanceMismatch,
    ExecuteFailed,
    StopFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePhase {
    Idempotency,
    Lifecycle,
    ArtifactVerification,
    Restore,
    GuestAuthentication,
    GenerationAcknowledgement,
    IdentityRepair,
    NetworkRepair,
    ReadinessProbe,
    Execute,
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Recovery {
    DoNotRetry,
    ReplaceMachine,
    RecertifyGeneration,
    RepairHost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupEvidence {
    NotRequired,
    Complete,
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    kind: FailureKind,
    phase: FailurePhase,
    recovery: Recovery,
    milestones: Milestones,
    cleanup: CleanupEvidence,
}

impl Failure {
    pub(crate) const fn new(
        kind: FailureKind,
        phase: FailurePhase,
        recovery: Recovery,
        milestones: Milestones,
        cleanup: CleanupEvidence,
    ) -> Self {
        Self {
            kind,
            phase,
            recovery,
            milestones,
            cleanup,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> FailureKind {
        self.kind
    }

    #[must_use]
    pub const fn phase(&self) -> FailurePhase {
        self.phase
    }

    #[must_use]
    pub const fn recovery(&self) -> Recovery {
        self.recovery
    }

    #[must_use]
    pub fn milestones(&self) -> &[crate::Milestone] {
        self.milestones.as_slice()
    }

    #[must_use]
    pub const fn cleanup(&self) -> CleanupEvidence {
        self.cleanup
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?} during {:?}", self.kind, self.phase)
    }
}

impl Error for Failure {}
