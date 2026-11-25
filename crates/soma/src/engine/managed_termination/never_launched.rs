//! The destroy of an Instance whose launch never finished (SOMA#220).
//!
//! A launch left launching when its host restarted under it, and a launch that failed, answered
//! every destroy with 409 for good, so the caller could never let go of the identity. A launch
//! left launching is rolled back exactly as a retry of that launch would roll it back; one that
//! failed already released the host. Either way the record becomes a destroy, so the same request
//! replays and a new create may take the identity.

use crate::{
    Backend, CleanupEvidence, CleanupReason, Milestone, MilestoneKind, Observation,
    ObservationUnavailable, OperationKind, StateStore, TerminalStatus, WorkloadEvidence,
};

use crate::engine::{
    Engine, ManagedFailure, RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, LaunchIntent, TerminalBasis},
    managed_receipt::{ManagedLaunch, ManagedReceipt, managed_receipt, operation_failure},
};

use super::{mode::TerminationMode, receipt::store_operation_failure};

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn destroy_never_launched(
        &mut self,
        operation_id: crate::OperationId,
        instance_id: &crate::InstanceId,
        revision: crate::StateRevision,
        intent: LaunchIntent,
        cleaned: bool,
    ) -> Result<crate::ExecutionReceipt, ManagedFailure> {
        let mode = TerminationMode::Forced;
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];
        let (complete, evidence, failure_kind) = if cleaned {
            milestones.push(Milestone::new(MilestoneKind::CleanupStarted, 0));
            milestones.push(Milestone::new(MilestoneKind::CleanupFinished, 0));
            (true, CleanupEvidence::not_owned(), None)
        } else {
            let cleanup = self.perform_cleanup(
                &operation_id,
                instance_id,
                CleanupReason::ForcedDestroy,
                &mut milestones,
            );
            (cleanup.complete, cleanup.evidence, cleanup.failure_kind)
        };
        let fingerprint = mode.fingerprint(&intent.workload, instance_id);
        let receipt = managed_receipt(ManagedReceipt {
            operation: OperationKind::Destroy,
            operation_id: operation_id.clone(),
            instance_id: instance_id.clone(),
            machine_name: intent.machine_name.clone(),
            fingerprint: fingerprint.clone(),
            workload: WorkloadEvidence::Resolved {
                identity: intent.workload.clone(),
            },
            backend: intent.backend,
            launch: ManagedLaunch::NotReached,
            shape: intent.requested_shape.clone(),
            milestones,
            terminal: if complete {
                mode.terminal_status()
            } else {
                TerminalStatus::Failed
            },
            output: Observation::Unavailable(ObservationUnavailable::NotReached),
            cleanup: evidence,
            measurement: mode.measurement(),
        });
        if !complete {
            // The half-built machine is still owned, so the record stays launching and a later
            // destroy retries the rollback.
            return Err(ManagedFailure::operation(operation_failure(
                failure_kind.unwrap_or(RunFailureKind::CleanupIncomplete),
                receipt,
                None,
            )));
        }
        let mut terminal = DurableMachine::launching(instance_id.clone(), intent.clone());
        terminal.phase = DurablePhase::Terminal {
            basis: Box::new(TerminalBasis::Launch {
                intent: Box::new(intent),
            }),
            operation: OperationKind::Destroy,
            operation_id,
            request_fingerprint: fingerprint,
            receipt: Box::new(receipt.clone()),
        };
        if let Err(failure) = self.replace_machine(revision, &terminal) {
            return Err(store_operation_failure(&failure, receipt));
        }
        Ok(receipt)
    }
}
