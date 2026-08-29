use crate::{
    BackendKind, EffectiveNetwork, InstanceId, MachineShape, MachineState, OperationId,
    WorkloadIdentity,
};

#[derive(Clone, Copy)]
pub struct InspectionRequest<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    workload: &'a WorkloadIdentity,
    shape: &'a MachineShape,
}

impl<'a> InspectionRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        workload: &'a WorkloadIdentity,
        shape: &'a MachineShape,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            workload,
            shape,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn workload(&self) -> &WorkloadIdentity {
        self.workload
    }

    #[must_use]
    pub const fn shape(&self) -> &MachineShape {
        self.shape
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectionObservation {
    operation_id: OperationId,
    instance_id: InstanceId,
    workload: WorkloadIdentity,
    backend: BackendKind,
    state: MachineState,
    effective_network: EffectiveNetwork,
    observed_at_ns: u64,
}

pub(crate) struct InspectionObservationParts {
    pub(crate) operation_id: OperationId,
    pub(crate) instance_id: InstanceId,
    pub(crate) workload: WorkloadIdentity,
    pub(crate) backend: BackendKind,
    pub(crate) state: MachineState,
    pub(crate) effective_network: EffectiveNetwork,
    pub(crate) observed_at_ns: u64,
}

impl InspectionObservation {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        workload: WorkloadIdentity,
        backend: BackendKind,
        state: MachineState,
        effective_network: EffectiveNetwork,
        observed_at_ns: u64,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            workload,
            backend,
            state,
            effective_network,
            observed_at_ns,
        }
    }

    pub(crate) fn into_parts(self) -> InspectionObservationParts {
        InspectionObservationParts {
            operation_id: self.operation_id,
            instance_id: self.instance_id,
            workload: self.workload,
            backend: self.backend,
            state: self.state,
            effective_network: self.effective_network,
            observed_at_ns: self.observed_at_ns,
        }
    }
}
