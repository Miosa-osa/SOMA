use crate::{CleanupReason, MeasurementClass, OperationKind, TerminalStatus};

#[derive(Clone, Copy)]
pub(super) enum TerminationMode {
    Graceful,
    Forced,
}

impl TerminationMode {
    pub(super) const fn operation(self) -> OperationKind {
        match self {
            Self::Graceful => OperationKind::Stop,
            Self::Forced => OperationKind::Destroy,
        }
    }

    pub(super) const fn cleanup_reason(self) -> CleanupReason {
        match self {
            Self::Graceful => CleanupReason::GracefulStop,
            Self::Forced => CleanupReason::ForcedDestroy,
        }
    }

    pub(super) const fn terminal_status(self) -> TerminalStatus {
        match self {
            Self::Graceful => TerminalStatus::Stopped,
            Self::Forced => TerminalStatus::Destroyed,
        }
    }

    pub(super) const fn measurement(self) -> MeasurementClass {
        match self {
            Self::Graceful => MeasurementClass::FacadeManagedStop,
            Self::Forced => MeasurementClass::FacadeManagedDestroy,
        }
    }

    pub(super) fn fingerprint(
        self,
        workload: &crate::WorkloadIdentity,
        instance_id: &crate::InstanceId,
    ) -> crate::RequestFingerprint {
        match self {
            Self::Graceful => crate::fingerprint::stop(workload, instance_id),
            Self::Forced => crate::fingerprint::destroy(workload, instance_id),
        }
    }
}
