use std::{error::Error, fmt};

use soma::StateStoreFailureKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalFailureKind {
    InvalidConfiguration,
    UnsupportedTarget,
    BackendUnavailable,
    StateStore(StateStoreFailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalFailure {
    kind: LocalFailureKind,
}

impl LocalFailure {
    #[must_use]
    pub const fn new(kind: LocalFailureKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> LocalFailureKind {
        self.kind
    }
}

impl fmt::Display for LocalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("local SOMA runtime setup failed")
    }
}

impl Error for LocalFailure {}
