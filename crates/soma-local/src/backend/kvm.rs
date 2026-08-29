use soma::{BackendFailure, BackendFailureKind, BackendKind, OperationId};

use super::clock::OperationClocks;

pub(crate) struct KvmBackend {
    clocks: OperationClocks,
}

impl KvmBackend {
    pub(super) fn new() -> Self {
        Self {
            clocks: OperationClocks::new(),
        }
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::LinuxKvm
    }

    pub(super) fn unavailable(&mut self, operation_id: &OperationId) -> BackendFailure {
        BackendFailure::new(
            BackendFailureKind::Unsupported,
            self.clocks.elapsed_ns(operation_id),
        )
    }
}
