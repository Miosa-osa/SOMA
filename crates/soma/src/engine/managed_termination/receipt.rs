use crate::{
    CleanupEvidence, Milestone, Observation, ObservationUnavailable, OperationKind, TerminalStatus,
    WorkloadEvidence,
};

use crate::engine::{
    ManagedFailure, RunFailureKind,
    machine_state::{ActiveMachine, DurableMachine, DurablePhase, TerminalBasis},
    managed_receipt::{ManagedLaunch, ManagedReceipt, managed_receipt, operation_failure},
};

use super::mode::TerminationMode;

#[allow(
    clippy::too_many_arguments,
    reason = "receipt assembly keeps every evidence dimension explicit at the schema boundary"
)]
pub(super) fn termination_receipt(
    operation_id: &crate::OperationId,
    instance_id: &crate::InstanceId,
    active: &ActiveMachine,
    fingerprint: crate::RequestFingerprint,
    mode: TerminationMode,
    milestones: Vec<Milestone>,
    terminal: TerminalStatus,
    cleanup: CleanupEvidence,
) -> crate::ExecutionReceipt {
    managed_receipt(ManagedReceipt {
        operation: mode.operation(),
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
        output: Observation::Unavailable(ObservationUnavailable::NotReached),
        cleanup,
        measurement: mode.measurement(),
    })
}

pub(super) fn durable_with_terminal(
    instance_id: &crate::InstanceId,
    active: ActiveMachine,
    operation: OperationKind,
    operation_id: crate::OperationId,
    request_fingerprint: crate::RequestFingerprint,
    receipt: crate::ExecutionReceipt,
) -> DurableMachine {
    let mut machine = DurableMachine::active(instance_id.clone(), active.launch_receipt.clone());
    machine.phase = DurablePhase::Terminal {
        basis: Box::new(TerminalBasis::Active {
            active: Box::new(active),
        }),
        operation,
        operation_id,
        request_fingerprint,
        receipt: Box::new(receipt),
    };
    machine
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
