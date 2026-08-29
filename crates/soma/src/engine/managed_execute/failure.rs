use crate::{
    Backend, CapturedOutput, CleanupReason, CommandStatus, Milestone, Observation,
    ObservationUnavailable, OperationKind, StateStore, TerminalStatus,
};

use super::{
    admission::ExecutionAdmission,
    durable_with_phase,
    receipt::{execution_receipt, store_operation_failure},
};
use crate::engine::{
    Engine, ExecuteMachineRequest, ManagedFailure, RunFailureKind,
    machine_state::{ActiveMachine, DurablePhase, TerminalBasis},
    managed_receipt::operation_failure,
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    #[allow(
        clippy::too_many_arguments,
        reason = "failed execution retains transaction and output evidence"
    )]
    pub(super) fn fail_execution(
        &mut self,
        request: &ExecuteMachineRequest,
        admission: ExecutionAdmission,
        mut milestones: Vec<Milestone>,
        kind: RunFailureKind,
        observed: Option<(CapturedOutput, crate::OutputMetadata, CommandStatus)>,
    ) -> ManagedFailure {
        let cleanup = self.perform_cleanup(
            &request.operation_id,
            &request.instance_id,
            CleanupReason::UncertainCommandTermination,
            &mut milestones,
        );
        let (output, output_evidence, terminal) = observed.map_or_else(
            || {
                (
                    None,
                    Observation::Unavailable(ObservationUnavailable::NotReached),
                    TerminalStatus::Failed,
                )
            },
            |(output, metadata, status)| {
                (
                    Some(output),
                    Observation::Observed(metadata),
                    crate::engine::run_evidence::terminal_status(status),
                )
            },
        );
        let receipt = execution_receipt(
            &request.operation_id,
            &request.instance_id,
            &admission.active,
            admission.fingerprint.clone(),
            milestones,
            terminal,
            output_evidence,
            cleanup.evidence,
        );
        if cleanup.complete {
            let terminal = durable_with_phase(
                &request.instance_id,
                DurablePhase::Terminal {
                    basis: Box::new(TerminalBasis::Active {
                        active: Box::new(admission.active),
                    }),
                    operation: OperationKind::Execute,
                    operation_id: request.operation_id.clone(),
                    request_fingerprint: admission.fingerprint,
                    receipt: Box::new(receipt.clone()),
                },
            );
            if let Err(failure) = self.replace_machine(admission.revision, &terminal) {
                return store_operation_failure(&failure, receipt, output);
            }
        }
        ManagedFailure::operation(operation_failure(
            cleanup.failure_kind.unwrap_or(kind),
            receipt,
            output,
        ))
    }

    pub(in crate::engine) fn recover_interrupted_execution(
        &mut self,
        revision: crate::StateRevision,
        instance_id: &crate::InstanceId,
        active: ActiveMachine,
        operation_id: crate::OperationId,
        request_fingerprint: crate::RequestFingerprint,
    ) -> ManagedFailure {
        let mut milestones = vec![crate::Milestone::new(crate::MilestoneKind::Accepted, 0)];
        let cleanup = self.perform_cleanup(
            &operation_id,
            instance_id,
            CleanupReason::UncertainCommandTermination,
            &mut milestones,
        );
        let receipt = execution_receipt(
            &operation_id,
            instance_id,
            &active,
            request_fingerprint.clone(),
            milestones,
            TerminalStatus::Failed,
            Observation::Unavailable(ObservationUnavailable::NotReached),
            cleanup.evidence,
        );
        if cleanup.complete {
            let terminal = durable_with_phase(
                instance_id,
                DurablePhase::Terminal {
                    basis: Box::new(TerminalBasis::Active {
                        active: Box::new(active),
                    }),
                    operation: OperationKind::Execute,
                    operation_id,
                    request_fingerprint,
                    receipt: Box::new(receipt.clone()),
                },
            );
            if let Err(failure) = self.replace_machine(revision, &terminal) {
                return store_operation_failure(&failure, receipt, None);
            }
        }
        ManagedFailure::operation(operation_failure(
            cleanup.failure_kind.unwrap_or(RunFailureKind::Interrupted),
            receipt,
            None,
        ))
    }
}
