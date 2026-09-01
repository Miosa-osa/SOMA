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

impl FailureKind {
    /// The kind's name on the wire, which is the name it has in this source.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OperationConflict => "OperationConflict",
            Self::OperationCapacityExceeded => "OperationCapacityExceeded",
            Self::InvalidLifecycle => "InvalidLifecycle",
            Self::GenerationVerificationFailed => "GenerationVerificationFailed",
            Self::RestoreFailed => "RestoreFailed",
            Self::GuestAuthenticationFailed => "GuestAuthenticationFailed",
            Self::GenerationAcknowledgementFailed => "GenerationAcknowledgementFailed",
            Self::IdentityRepairFailed => "IdentityRepairFailed",
            Self::NetworkRepairFailed => "NetworkRepairFailed",
            Self::ReadinessProbeFailed => "ReadinessProbeFailed",
            Self::InstanceMismatch => "InstanceMismatch",
            Self::ExecuteFailed => "ExecuteFailed",
            Self::StopFailed => "StopFailed",
        }
    }

    /// The kind one wire name is, or `None` for a name this contract does not define.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        const ALL: [FailureKind; 13] = [
            FailureKind::OperationConflict,
            FailureKind::OperationCapacityExceeded,
            FailureKind::InvalidLifecycle,
            FailureKind::GenerationVerificationFailed,
            FailureKind::RestoreFailed,
            FailureKind::GuestAuthenticationFailed,
            FailureKind::GenerationAcknowledgementFailed,
            FailureKind::IdentityRepairFailed,
            FailureKind::NetworkRepairFailed,
            FailureKind::ReadinessProbeFailed,
            FailureKind::InstanceMismatch,
            FailureKind::ExecuteFailed,
            FailureKind::StopFailed,
        ];
        ALL.into_iter().find(|kind| kind.name() == name)
    }
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
