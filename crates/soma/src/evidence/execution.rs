use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Exited { code: i32 },
    Signaled { signal: Option<i32> },
    TimedOut,
    OutputLimitExceeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneKind {
    Accepted,
    WorkloadResolved,
    Admitted,
    MachineLaunched,
    Ready,
    CommandStarted,
    CommandFinished,
    CleanupStarted,
    CleanupFinished,
    FailureObserved,
    Inspected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Milestone {
    kind: MilestoneKind,
    elapsed_ns: u64,
}

impl Milestone {
    pub(crate) const fn new(kind: MilestoneKind, elapsed_ns: u64) -> Self {
        Self { kind, elapsed_ns }
    }

    #[must_use]
    pub const fn kind(&self) -> MilestoneKind {
        self.kind
    }

    #[must_use]
    pub const fn elapsed_ns(&self) -> u64 {
        self.elapsed_ns
    }
}
