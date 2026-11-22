use crate::{Backend, CleanupReason, Milestone, OperationKind, StateStore, TerminalStatus};

use crate::engine::{
    Engine, LaunchMachineRequest, ManagedFailure, RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, LaunchIntent, TerminalBasis},
    managed_receipt::{ManagedLaunch, operation_failure},
};

use super::receipt::{launch_receipt, store_operation_failure};

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn reconcile_checkpoint_launch(
        &mut self,
        intent: &LaunchIntent,
        request: &LaunchMachineRequest,
        checkpoint: Option<&crate::OperationId>,
    ) -> bool {
        checkpoint.is_none_or(|checkpoint| {
            self.backend
                .reconcile_checkpoint_launch(
                    &request.instance_id,
                    &intent.operation_id,
                    checkpoint,
                    intent.workload.generation_id(),
                )
                .is_ok()
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "failed launch preserves its complete transaction context"
    )]
    pub(super) fn fail_launch(
        &mut self,
        request: &LaunchMachineRequest,
        intent: LaunchIntent,
        revision: crate::StateRevision,
        prior_stop: Option<Box<DurableMachine>>,
        checkpoint: Option<&crate::OperationId>,
        fingerprint: crate::RequestFingerprint,
        workload: crate::WorkloadIdentity,
        mut milestones: Vec<Milestone>,
        kind: RunFailureKind,
    ) -> ManagedFailure {
        let cleanup = self.perform_cleanup(
            &request.operation_id,
            &request.instance_id,
            CleanupReason::Rollback,
            &mut milestones,
        );
        let receipt = launch_receipt(
            request,
            fingerprint.clone(),
            crate::WorkloadEvidence::Resolved { identity: workload },
            self.backend.kind(),
            ManagedLaunch::NotReached,
            milestones,
            cleanup.evidence,
            TerminalStatus::Failed,
        );
        if cleanup.complete && self.reconcile_checkpoint_launch(&intent, request, checkpoint) {
            let terminal = prior_stop.map_or_else(
                || {
                    let mut terminal =
                        DurableMachine::launching(request.instance_id.clone(), intent.clone());
                    terminal.phase = DurablePhase::Terminal {
                        basis: Box::new(TerminalBasis::Launch {
                            intent: Box::new(intent),
                        }),
                        operation: OperationKind::Launch,
                        operation_id: request.operation_id.clone(),
                        request_fingerprint: fingerprint,
                        receipt: Box::new(receipt.clone()),
                    };
                    terminal
                },
                |stopped| *stopped,
            );
            if let Err(failure) = self.replace_machine(revision, &terminal) {
                return store_operation_failure(&failure, receipt);
            }
        }
        ManagedFailure::operation(operation_failure(
            cleanup.failure_kind.unwrap_or(kind),
            receipt,
            None,
        ))
    }
}
