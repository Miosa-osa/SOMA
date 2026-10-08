use serde::{Deserialize, Serialize};

/// The portable outcome of one command the machine was asked to run.
///
/// Every variant but the last is the result of a process that existed: it exited, a signal
/// ended it, its deadline expired, or its output allowance ran out. [`Self::SpawnFailed`] is
/// not a result, it is a refusal, and it is here rather than absent so that a caller can tell
/// "the program never started" apart from "the machine could not be reached". Without that
/// distinction the only status left to report was none at all, and a request the machine
/// simply would not run became an uncertain termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Exited {
        code: i32,
    },
    Signaled {
        signal: Option<i32>,
    },
    TimedOut,
    OutputLimitExceeded,
    /// The program could not be started, so no process ran. This is the positive Linux errno
    /// `execve` reported.
    SpawnFailed {
        errno: i32,
    },
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
