//! The request and result of a managed suspension (ADR 0034).

use std::path::PathBuf;

use crate::{ExecutionReceipt, InstanceId, OperationId};

/// Suspend one exact Instance, staging its members in `staging` for the Suspension store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuspendMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) staging: PathBuf,
}

impl SuspendMachineRequest {
    #[must_use]
    pub const fn new(operation_id: OperationId, instance_id: InstanceId, staging: PathBuf) -> Self {
        Self {
            operation_id,
            instance_id,
            staging,
        }
    }
}

/// A suspended Instance's exact replay receipt; its status is `Suspended` or `Failed`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineSuspend {
    pub(super) receipt: ExecutionReceipt,
}

impl MachineSuspend {
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}
