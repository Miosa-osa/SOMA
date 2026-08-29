use crate::{
    CleanupEvidence, MeasurementClass, Milestone, Observation, ObservationUnavailable,
    OperationKind, TerminalStatus, WorkloadEvidence,
};

use crate::engine::{
    LaunchMachineRequest, ManagedFailure, RunFailureKind,
    managed_receipt::{ManagedLaunch, ManagedReceipt, managed_receipt, operation_failure},
};

#[allow(
    clippy::too_many_arguments,
    reason = "receipt construction is a schema boundary"
)]
pub(super) fn launch_receipt(
    request: &LaunchMachineRequest,
    fingerprint: crate::RequestFingerprint,
    workload: WorkloadEvidence,
    backend: crate::BackendKind,
    launch: ManagedLaunch,
    milestones: Vec<Milestone>,
    cleanup: CleanupEvidence,
    terminal: TerminalStatus,
) -> crate::ExecutionReceipt {
    managed_receipt(ManagedReceipt {
        operation: OperationKind::Launch,
        operation_id: request.operation_id.clone(),
        instance_id: request.instance_id.clone(),
        machine_name: request.machine_name.clone(),
        fingerprint,
        workload,
        backend,
        launch,
        shape: request.shape.clone(),
        milestones,
        terminal,
        output: Observation::Unavailable(ObservationUnavailable::NotReached),
        cleanup,
        measurement: MeasurementClass::FacadeManagedLaunch,
    })
}

pub(super) fn store_operation_failure(
    failure: &ManagedFailure,
    receipt: crate::ExecutionReceipt,
) -> ManagedFailure {
    let kind = match failure {
        ManagedFailure::StateStore(kind) => *kind,
        _ => crate::StateStoreFailureKind::Corrupt,
    };
    ManagedFailure::operation(operation_failure(
        RunFailureKind::StateStore { kind },
        receipt,
        None,
    ))
}
