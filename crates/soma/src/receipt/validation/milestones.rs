//! Which milestones each operation may record, and their order.

use crate::{MilestoneKind, OperationKind};

pub(super) fn milestone_allowed(operation: OperationKind, kind: MilestoneKind) -> bool {
    match operation {
        OperationKind::Run => kind != MilestoneKind::Inspected,
        OperationKind::Launch => matches!(
            kind,
            MilestoneKind::Accepted
                | MilestoneKind::WorkloadResolved
                | MilestoneKind::Admitted
                | MilestoneKind::MachineLaunched
                | MilestoneKind::Ready
                | MilestoneKind::FailureObserved
                | MilestoneKind::CleanupStarted
                | MilestoneKind::CleanupFinished
        ),
        OperationKind::Execute => matches!(
            kind,
            MilestoneKind::Accepted
                | MilestoneKind::CommandStarted
                | MilestoneKind::CommandFinished
                | MilestoneKind::FailureObserved
                | MilestoneKind::CleanupStarted
                | MilestoneKind::CleanupFinished
        ),
        OperationKind::Stop | OperationKind::Destroy | OperationKind::Suspend => matches!(
            kind,
            MilestoneKind::Accepted
                | MilestoneKind::CleanupStarted
                | MilestoneKind::CleanupFinished
                | MilestoneKind::FailureObserved
        ),
        OperationKind::Inspect => matches!(
            kind,
            MilestoneKind::Accepted | MilestoneKind::Inspected | MilestoneKind::FailureObserved
        ),
    }
}

pub(super) const fn milestone_rank(kind: MilestoneKind) -> u8 {
    match kind {
        MilestoneKind::Accepted => 0,
        MilestoneKind::WorkloadResolved | MilestoneKind::Inspected => 1,
        MilestoneKind::Admitted => 2,
        MilestoneKind::MachineLaunched => 3,
        MilestoneKind::Ready => 4,
        MilestoneKind::CommandStarted => 5,
        MilestoneKind::CommandFinished => 6,
        MilestoneKind::CleanupStarted => 7,
        MilestoneKind::CleanupFinished => 8,
        MilestoneKind::FailureObserved => 9,
    }
}

pub(super) const fn milestone_slot(kind: MilestoneKind) -> usize {
    match kind {
        MilestoneKind::Accepted => 0,
        MilestoneKind::WorkloadResolved => 1,
        MilestoneKind::Admitted => 2,
        MilestoneKind::MachineLaunched => 3,
        MilestoneKind::Ready => 4,
        MilestoneKind::CommandStarted => 5,
        MilestoneKind::CommandFinished => 6,
        MilestoneKind::CleanupStarted => 7,
        MilestoneKind::CleanupFinished => 8,
        MilestoneKind::FailureObserved => 9,
        MilestoneKind::Inspected => 10,
    }
}
