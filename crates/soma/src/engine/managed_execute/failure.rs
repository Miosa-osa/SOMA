use crate::{
    Backend, CapturedOutput, CleanupEvidence, CleanupReason, CommandStatus, Milestone, Observation,
    ObservationUnavailable, OperationKind, StateStore, TerminalStatus,
};

use super::{
    admission::ExecutionAdmission,
    disposition::CommandDisposition,
    durable_with_phase,
    receipt::{execution_receipt, store_operation_failure},
};
use crate::engine::{
    Engine, ExecuteMachineRequest, ManagedFailure, RunFailureKind,
    machine_state::{ActiveMachine, DurablePhase, TerminalBasis},
    managed_receipt::operation_failure,
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Records one failed execution and settles what happens to the machine that ran it.
    ///
    /// A disposition that keeps the machine leaves it owned and ready: no cleanup is asked for,
    /// no terminal phase is written, and the durable state goes back to the active phase the
    /// admission found. The failure still carries its receipt, so a caller reads what happened
    /// and the sandbox it was aimed at is still there to be asked again.
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
        disposition: CommandDisposition,
    ) -> ManagedFailure {
        // Only a machine nobody observed is released. The rest were never in doubt, and asking
        // for their cleanup is the whole of the defect this disposition exists to prevent.
        let cleanup = (!disposition.keeps_the_machine()).then(|| {
            self.perform_cleanup(
                &request.operation_id,
                &request.instance_id,
                CleanupReason::UncertainCommandTermination,
                &mut milestones,
            )
        });
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
        // A machine that is kept performed no cleanup, and its receipt says so rather than
        // claiming resources it still owns were released.
        let (evidence, released, cleanup_kind) = match cleanup {
            Some(cleanup) => (cleanup.evidence, cleanup.complete, cleanup.failure_kind),
            None => (CleanupEvidence::not_owned(), false, None),
        };
        let receipt = execution_receipt(
            &request.operation_id,
            &request.instance_id,
            &admission.active,
            admission.fingerprint.clone(),
            milestones,
            terminal,
            output_evidence,
            evidence,
        );
        if let Err(failure) = self.settle_machine(request, admission, &receipt, released) {
            return store_operation_failure(&failure, receipt, output);
        }
        ManagedFailure::operation(operation_failure(
            cleanup_kind.unwrap_or(kind),
            receipt,
            output,
        ))
    }

    /// Moves the machine's durable phase to wherever this failure leaves it.
    ///
    /// A released machine gets its terminal phase, with the receipt as the durable record of why.
    /// A kept one goes back to active, which is where it was before the operation was admitted.
    fn settle_machine(
        &mut self,
        request: &ExecuteMachineRequest,
        admission: ExecutionAdmission,
        receipt: &crate::ExecutionReceipt,
        released: bool,
    ) -> Result<(), ManagedFailure> {
        let phase = if released {
            DurablePhase::Terminal {
                basis: Box::new(TerminalBasis::Active {
                    active: Box::new(admission.active),
                }),
                operation: OperationKind::Execute,
                operation_id: request.operation_id.clone(),
                request_fingerprint: admission.fingerprint,
                receipt: Box::new(receipt.clone()),
            }
        } else {
            DurablePhase::Active {
                active: Box::new(admission.active),
            }
        };
        let next = durable_with_phase(&request.instance_id, phase);
        self.replace_machine(admission.revision, &next).map(|_| ())
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
