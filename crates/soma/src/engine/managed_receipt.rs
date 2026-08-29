use crate::{
    CapturedOutput, CleanupEvidence, EffectiveNetwork, EffectiveShape, ExecutionReceipt,
    InstanceId, MeasurementBoundary, MeasurementClass, Milestone, Observation,
    ObservationUnavailable, OperationId, OperationKind, RunFailure, RunFailureKind, TerminalStatus,
    WorkloadEvidence,
};

use super::run_evidence::LaunchEvidence;

pub(super) enum ManagedLaunch {
    NotReached,
    Fresh(LaunchEvidence),
    Stored(Box<ExecutionReceipt>),
    Inspected {
        receipt: Box<ExecutionReceipt>,
        effective_network: EffectiveNetwork,
    },
}

pub(super) struct ManagedReceipt {
    pub(super) operation: OperationKind,
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) machine_name: Option<crate::MachineName>,
    pub(super) fingerprint: crate::RequestFingerprint,
    pub(super) workload: WorkloadEvidence,
    pub(super) backend: crate::BackendKind,
    pub(super) launch: ManagedLaunch,
    pub(super) shape: crate::MachineShape,
    pub(super) milestones: Vec<Milestone>,
    pub(super) terminal: TerminalStatus,
    pub(super) output: Observation<crate::OutputMetadata>,
    pub(super) cleanup: CleanupEvidence,
    pub(super) measurement: MeasurementClass,
}

pub(super) fn managed_receipt(context: ManagedReceipt) -> ExecutionReceipt {
    let (isolation, preparation, binding, effective_shape, effective_network) = match context.launch
    {
        ManagedLaunch::NotReached => (
            Observation::Unavailable(ObservationUnavailable::NotReached),
            Observation::Unavailable(ObservationUnavailable::NotReached),
            Observation::Unavailable(ObservationUnavailable::NotReached),
            EffectiveShape::unavailable(ObservationUnavailable::NotReached),
            EffectiveNetwork::unavailable(ObservationUnavailable::NotReached),
        ),
        ManagedLaunch::Fresh(launch) => (
            Observation::Observed(launch.isolation),
            Observation::Observed(launch.preparation),
            Observation::Observed(launch.digest_binding),
            launch.effective_shape,
            launch.effective_network,
        ),
        ManagedLaunch::Stored(receipt) => (
            receipt.isolation().clone(),
            receipt.preparation().clone(),
            receipt.digest_binding().clone(),
            receipt.effective_shape().clone(),
            receipt.effective_network().clone(),
        ),
        ManagedLaunch::Inspected {
            receipt,
            effective_network,
        } => (
            receipt.isolation().clone(),
            receipt.preparation().clone(),
            receipt.digest_binding().clone(),
            receipt.effective_shape().clone(),
            effective_network,
        ),
    };
    ExecutionReceipt::new(
        context.operation,
        context.operation_id,
        context.instance_id,
        context.machine_name,
        context.fingerprint,
        context.workload,
        context.backend,
        isolation,
        preparation,
        binding,
        context.shape,
        effective_shape,
        effective_network,
        context.milestones,
        context.terminal,
        context.output,
        context.cleanup,
        MeasurementBoundary::for_class(context.measurement),
    )
}

pub(super) fn operation_failure(
    kind: RunFailureKind,
    receipt: ExecutionReceipt,
    output: Option<CapturedOutput>,
) -> RunFailure {
    RunFailure {
        kind,
        receipt: Box::new(receipt),
        output,
    }
}
