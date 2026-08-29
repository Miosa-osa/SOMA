use crate::{
    BackendKind, DigestBinding, EffectiveNetwork, EffectiveShape, InstanceId, IsolationClass,
    MachineShape, OperationId, PreparationClass, WorkloadIdentity,
};

#[derive(Clone, Copy)]
pub struct LaunchRequest<'a, P> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    workload: &'a WorkloadIdentity,
    prepared: &'a P,
    shape: &'a MachineShape,
}

impl<'a, P> LaunchRequest<'a, P> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        workload: &'a WorkloadIdentity,
        prepared: &'a P,
        shape: &'a MachineShape,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            workload,
            prepared,
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
    pub const fn prepared(&self) -> &P {
        self.prepared
    }

    #[must_use]
    pub const fn shape(&self) -> &MachineShape {
        self.shape
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchTimes {
    admitted: u64,
    launched: u64,
    ready: u64,
}

impl LaunchTimes {
    #[must_use]
    pub const fn new(admitted_at_ns: u64, launched_at_ns: u64, ready_at_ns: u64) -> Self {
        Self {
            admitted: admitted_at_ns,
            launched: launched_at_ns,
            ready: ready_at_ns,
        }
    }

    pub(crate) const fn values(self) -> [u64; 3] {
        [self.admitted, self.launched, self.ready]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchObservation {
    operation_id: OperationId,
    instance_id: InstanceId,
    workload: WorkloadIdentity,
    backend: BackendKind,
    isolation: IsolationClass,
    preparation: PreparationClass,
    digest_binding: DigestBinding,
    effective_shape: EffectiveShape,
    effective_network: EffectiveNetwork,
    times: LaunchTimes,
}

pub(crate) struct LaunchObservationParts {
    pub(crate) operation_id: OperationId,
    pub(crate) instance_id: InstanceId,
    pub(crate) workload: WorkloadIdentity,
    pub(crate) backend: BackendKind,
    pub(crate) isolation: IsolationClass,
    pub(crate) preparation: PreparationClass,
    pub(crate) digest_binding: DigestBinding,
    pub(crate) effective_shape: EffectiveShape,
    pub(crate) effective_network: EffectiveNetwork,
    pub(crate) times: LaunchTimes,
}

impl LaunchObservation {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        workload: WorkloadIdentity,
        backend: BackendKind,
        isolation: IsolationClass,
        preparation: PreparationClass,
        digest_binding: DigestBinding,
        effective_shape: EffectiveShape,
        effective_network: EffectiveNetwork,
        times: LaunchTimes,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            workload,
            backend,
            isolation,
            preparation,
            digest_binding,
            effective_shape,
            effective_network,
            times,
        }
    }

    pub(crate) fn into_parts(self) -> LaunchObservationParts {
        LaunchObservationParts {
            operation_id: self.operation_id,
            instance_id: self.instance_id,
            workload: self.workload,
            backend: self.backend,
            isolation: self.isolation,
            preparation: self.preparation,
            digest_binding: self.digest_binding,
            effective_shape: self.effective_shape,
            effective_network: self.effective_network,
            times: self.times,
        }
    }
}
