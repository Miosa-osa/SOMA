use crate::{BackendTarget, InstanceId, OperationId};

macro_rules! control_request {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name {
            operation_id: OperationId,
            instance_id: InstanceId,
            backend: BackendTarget,
        }

        impl $name {
            pub(crate) const fn new(
                operation_id: OperationId,
                instance_id: InstanceId,
                backend: BackendTarget,
            ) -> Self {
                Self {
                    operation_id,
                    instance_id,
                    backend,
                }
            }

            #[must_use]
            pub const fn operation_id(&self) -> &OperationId {
                &self.operation_id
            }

            #[must_use]
            pub const fn instance_id(&self) -> &InstanceId {
                &self.instance_id
            }

            #[must_use]
            pub const fn backend(&self) -> BackendTarget {
                self.backend
            }
        }
    };
}

control_request!(InspectRequest);
control_request!(StopRequest);
control_request!(DestroyRequest);

impl InspectRequest {
    pub(crate) fn into_facade(self) -> soma::InspectMachineRequest {
        soma::InspectMachineRequest::new(self.operation_id, self.instance_id)
    }
}

impl StopRequest {
    pub(crate) fn into_facade(self) -> soma::StopMachineRequest {
        soma::StopMachineRequest::new(self.operation_id, self.instance_id)
    }
}

impl DestroyRequest {
    pub(crate) fn into_facade(self) -> soma::DestroyMachineRequest {
        soma::DestroyMachineRequest::new(self.operation_id, self.instance_id)
    }
}
