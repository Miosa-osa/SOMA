use crate::{Generation, InstanceId, OperationId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Launch {
    operation_id: OperationId,
    instance_id: InstanceId,
    generation: Generation,
}

impl Launch {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        generation: Generation,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            generation,
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
    pub const fn generation(&self) -> &Generation {
        &self.generation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stop {
    operation_id: OperationId,
    instance_id: InstanceId,
}

impl Stop {
    #[must_use]
    pub const fn new(operation_id: OperationId, instance_id: InstanceId) -> Self {
        Self {
            operation_id,
            instance_id,
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

    pub(crate) const fn into_ids(self) -> (OperationId, InstanceId) {
        (self.operation_id, self.instance_id)
    }
}
