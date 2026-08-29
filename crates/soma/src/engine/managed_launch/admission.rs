use crate::{Backend, CleanupReason, Milestone, MilestoneKind, OperationKind, StateStore};

use crate::engine::{
    Engine, LaunchMachineRequest, ManagedFailure, ManagedStateError, RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, LaunchIntent, TerminalBasis},
    managed::ReplayEvidence,
    managed_receipt::{ManagedLaunch, operation_failure},
    managed_state::is_store_conflict,
};

use super::receipt::launch_receipt;

pub(super) enum LaunchAdmission {
    Proceed(crate::StateRevision),
    Replay(Box<crate::ExecutionReceipt>),
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn admit_launch(
        &mut self,
        request: &LaunchMachineRequest,
        intent: &LaunchIntent,
        resolved_at: u64,
        milestones: &mut Vec<Milestone>,
    ) -> Result<LaunchAdmission, ManagedFailure> {
        let pending = DurableMachine::launching(request.instance_id.clone(), intent.clone());
        match self.create_machine(&pending) {
            Ok(revision) => return Ok(LaunchAdmission::Proceed(revision)),
            Err(failure) if is_store_conflict(&failure) => {}
            Err(failure) => return Err(failure),
        }
        let stored = self
            .load_machine(&request.instance_id)?
            .ok_or(ManagedFailure::StateStore(
                crate::StateStoreFailureKind::Conflict,
            ))?;
        match &stored.machine.phase {
            DurablePhase::Launching { intent: existing } if existing.as_ref() == intent => {
                let cleanup = self.perform_cleanup(
                    &existing.operation_id,
                    &request.instance_id,
                    CleanupReason::Rollback,
                    milestones,
                );
                if !cleanup.complete {
                    let receipt = launch_receipt(
                        request,
                        intent.request_fingerprint.clone(),
                        crate::WorkloadEvidence::Resolved {
                            identity: intent.workload.clone(),
                        },
                        intent.backend,
                        ManagedLaunch::NotReached,
                        milestones.clone(),
                        cleanup.evidence,
                        crate::TerminalStatus::Failed,
                    );
                    return Err(ManagedFailure::operation(operation_failure(
                        cleanup
                            .failure_kind
                            .unwrap_or(RunFailureKind::CleanupIncomplete),
                        receipt,
                        None,
                    )));
                }
                let revision = self.replace_machine(stored.revision, &pending)?;
                *milestones = vec![
                    Milestone::new(MilestoneKind::Accepted, 0),
                    Milestone::new(MilestoneKind::WorkloadResolved, resolved_at),
                ];
                Ok(LaunchAdmission::Proceed(revision))
            }
            DurablePhase::Active { active }
                if active.launch_receipt.operation_id() == &request.operation_id
                    && active.launch_receipt.request_fingerprint()
                        == &intent.request_fingerprint =>
            {
                Ok(LaunchAdmission::Replay(Box::new(
                    active.launch_receipt.clone(),
                )))
            }
            DurablePhase::Terminal {
                basis,
                operation: OperationKind::Launch,
                operation_id,
                request_fingerprint,
                receipt,
            } if matches!(
                basis.as_ref(),
                TerminalBasis::Launch { intent: existing } if existing.as_ref() == intent
            ) && operation_id == &request.operation_id
                && request_fingerprint == &intent.request_fingerprint =>
            {
                Err(ManagedFailure::ReplayUnavailable(
                    ReplayEvidence::from_receipt(receipt.as_ref().clone()),
                ))
            }
            DurablePhase::Active { .. } => Err(ManagedFailure::State(
                ManagedStateError::MachineAlreadyExists,
            )),
            _ => Err(ManagedFailure::State(ManagedStateError::OperationConflict)),
        }
    }
}
