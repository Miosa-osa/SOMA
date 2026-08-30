mod host;
mod prepared;
mod resolve;

use soma::{BackendFailure, BackendFailureKind, BackendKind, OperationId};

use super::clock::OperationClocks;

use host::HostInputs;

pub(crate) struct KvmBackend {
    clocks: OperationClocks,
    #[allow(
        dead_code,
        reason = "read once the lifecycle below stops failing closed"
    )]
    host: Option<HostInputs>,
}

impl KvmBackend {
    /// Opens the Backend and records whether this host carries the artifacts a Generation needs.
    ///
    /// Missing artifacts do not fail the open. Every lifecycle call still fails closed as
    /// unsupported, so reporting an unavailable capability here would claim the lifecycle exists
    /// and merely lacks its inputs. The recorded outcome becomes a real precondition in the same
    /// change that implements the lifecycle.
    pub(super) fn open() -> Self {
        Self {
            clocks: OperationClocks::new(),
            host: HostInputs::resolve().ok(),
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
