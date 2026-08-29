use crate::{BackendFailureKind, CapturedOutput, ExecutionReceipt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailurePhase {
    Resolution,
    Launch,
    Command,
    Cleanup,
    Inspect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunFailureKind {
    Backend {
        phase: FailurePhase,
        kind: BackendFailureKind,
    },
    ObservationMismatch,
    CleanupIncomplete,
    TimedOut,
    OutputLimitExceeded,
    Interrupted,
    StateStore {
        kind: crate::StateStoreFailureKind,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunFailure {
    pub(super) kind: RunFailureKind,
    pub(super) receipt: Box<ExecutionReceipt>,
    pub(super) output: Option<CapturedOutput>,
}

impl RunFailure {
    #[must_use]
    pub const fn kind(&self) -> RunFailureKind {
        self.kind
    }

    #[must_use]
    pub fn receipt(&self) -> &ExecutionReceipt {
        self.receipt.as_ref()
    }

    #[must_use]
    pub fn output(&self) -> Option<&CapturedOutput> {
        self.output.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    pub(super) receipt: ExecutionReceipt,
    pub(super) output: CapturedOutput,
}

impl RunOutcome {
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn output(&self) -> &CapturedOutput {
        &self.output
    }
}
