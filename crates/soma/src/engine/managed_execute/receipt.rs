use crate::{
    CapturedOutput, CleanupEvidence, MeasurementClass, Milestone, Observation, OperationKind,
    TerminalStatus, WorkloadEvidence,
};

use crate::engine::{
    ManagedFailure, RunFailureKind,
    machine_state::ActiveMachine,
    managed_receipt::{ManagedLaunch, ManagedReceipt, managed_receipt, operation_failure},
};

#[allow(
    clippy::too_many_arguments,
    reason = "receipt assembly keeps every evidence dimension explicit at the schema boundary"
)]
pub(super) fn execution_receipt(
    operation_id: &crate::OperationId,
    instance_id: &crate::InstanceId,
    active: &ActiveMachine,
    fingerprint: crate::RequestFingerprint,
    milestones: Vec<Milestone>,
    terminal: TerminalStatus,
    output: Observation<crate::OutputMetadata>,
    cleanup: CleanupEvidence,
) -> crate::ExecutionReceipt {
    managed_receipt(ManagedReceipt {
        operation: OperationKind::Execute,
        operation_id: operation_id.clone(),
        instance_id: instance_id.clone(),
        machine_name: active.machine_name().cloned(),
        fingerprint,
        workload: WorkloadEvidence::Resolved {
            identity: active
                .workload()
                .expect("validated active state has resolved workload")
                .clone(),
        },
        backend: active.backend(),
        launch: ManagedLaunch::Stored(Box::new(active.launch_receipt.clone())),
        shape: active.shape().clone(),
        milestones,
        terminal,
        output,
        cleanup,
        measurement: MeasurementClass::FacadeManagedCommand,
    })
}

pub(super) fn store_operation_failure(
    failure: &ManagedFailure,
    receipt: crate::ExecutionReceipt,
    output: Option<CapturedOutput>,
) -> ManagedFailure {
    let kind = match failure {
        ManagedFailure::StateStore(kind) => *kind,
        _ => crate::StateStoreFailureKind::Corrupt,
    };
    ManagedFailure::operation(operation_failure(
        RunFailureKind::StateStore { kind },
        receipt,
        output,
    ))
}
