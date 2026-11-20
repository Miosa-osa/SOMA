#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendFailureKind {
    Unsupported,
    Unavailable,
    ResourceConflict,
    WorkloadRejected,
    /// The sandbox's guest agent cannot decode this command (an argument longer than it carries),
    /// so it was refused with nothing sent; the sandbox is kept, and its Generation must be
    /// captured again with a newer agent for the command to run.
    GuestAgentOutdated,
    IsolationFailure,
    GuestFailure,
    Timeout,
    OutputLimit,
    CleanupFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendFailure {
    kind: BackendFailureKind,
    occurred_at_ns: u64,
}

impl BackendFailure {
    #[must_use]
    pub const fn new(kind: BackendFailureKind, occurred_at_ns: u64) -> Self {
        Self {
            kind,
            occurred_at_ns,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> BackendFailureKind {
        self.kind
    }

    #[must_use]
    pub const fn occurred_at_ns(&self) -> u64 {
        self.occurred_at_ns
    }
}
