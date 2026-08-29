use crate::{CleanupEvidence, InstanceId, OperationId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupReason {
    RunCompleted,
    Rollback,
    GracefulStop,
    ForcedDestroy,
    UncertainCommandTermination,
}

#[derive(Clone, Copy)]
pub struct CleanupRequest<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    reason: CleanupReason,
}

impl<'a> CleanupRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        reason: CleanupReason,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            reason,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn reason(&self) -> CleanupReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupTimes {
    started_at_ns: u64,
    finished_at_ns: u64,
}

impl CleanupTimes {
    #[must_use]
    pub const fn new(started_at_ns: u64, finished_at_ns: u64) -> Self {
        Self {
            started_at_ns,
            finished_at_ns,
        }
    }

    pub(crate) const fn values(self) -> [u64; 2] {
        [self.started_at_ns, self.finished_at_ns]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupObservation {
    operation_id: OperationId,
    instance_id: InstanceId,
    evidence: CleanupEvidence,
    times: CleanupTimes,
}

impl CleanupObservation {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        evidence: CleanupEvidence,
        times: CleanupTimes,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            evidence,
            times,
        }
    }

    pub(crate) fn into_parts(self) -> (OperationId, InstanceId, CleanupEvidence, CleanupTimes) {
        (
            self.operation_id,
            self.instance_id,
            self.evidence,
            self.times,
        )
    }
}
