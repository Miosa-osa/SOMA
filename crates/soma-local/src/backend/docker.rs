mod cleanup;
mod command;
mod container;
mod execute;
mod inspect;
mod launch;
mod network;
mod probe;
mod process;
mod resolve;

use soma::{BackendFailure, BackendFailureKind, BackendKind};

use super::{LocalFailure, clock::OperationClocks};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(super) use probe::is_available;
pub(super) use probe::probe;

pub(crate) struct DockerBackend {
    already_cleaned: std::collections::BTreeSet<String>,
    clocks: OperationClocks,
}

impl DockerBackend {
    pub(super) fn open() -> Result<Self, LocalFailure> {
        probe()?;
        Ok(Self {
            already_cleaned: std::collections::BTreeSet::new(),
            clocks: OperationClocks::new(),
        })
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::DockerContainer
    }
}

fn failure(operation: &soma::OperationId, kind: BackendFailureKind) -> BackendFailure {
    let _ = operation;
    BackendFailure::new(kind, 1)
}
