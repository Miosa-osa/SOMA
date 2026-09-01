//! One bounded terminal operation carried across the jailed worker channel.

use crate::{InstanceId, OperationId};

/// A terminal call bound to one operation and one live Instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtyRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    operation: soma::PtyOperation,
}

impl PtyRequest {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        operation: soma::PtyOperation,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            operation,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn operation(&self) -> &soma::PtyOperation {
        &self.operation
    }
}
