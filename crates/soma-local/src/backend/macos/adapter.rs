use std::{collections::BTreeSet, path::PathBuf};

use soma::{BackendKind, WorkloadIdentity};
use soma_macos::{BackendError, ImageReference, MacOsBackend};

use crate::backend::clock::OperationClocks;

use super::config::resolve_runtime;

pub(crate) struct MacPreparedWorkload {
    pub(super) image: ImageReference,
    pub(super) identity: WorkloadIdentity,
}

pub(crate) struct MacBackend {
    pub(super) backend: MacOsBackend,
    pub(super) clocks: OperationClocks,
    pub(super) already_cleaned: BTreeSet<String>,
    runtime_version: String,
}

impl MacBackend {
    pub(in crate::backend) fn open(
        explicit_runtime: Option<PathBuf>,
    ) -> Result<Self, BackendError> {
        let backend = MacOsBackend::with_executable(resolve_runtime(explicit_runtime));
        let report = backend.probe()?;
        Ok(Self {
            backend,
            clocks: OperationClocks::new(),
            already_cleaned: BTreeSet::new(),
            runtime_version: report.cli().version().to_owned(),
        })
    }

    pub(in crate::backend) const fn kind() -> BackendKind {
        BackendKind::MacosVirtualization
    }

    pub(in crate::backend) fn runtime_version(&self) -> &str {
        &self.runtime_version
    }
}
