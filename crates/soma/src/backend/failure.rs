#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendFailureKind {
    Unsupported,
    Unavailable,
    ResourceConflict,
    WorkloadRejected,
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
