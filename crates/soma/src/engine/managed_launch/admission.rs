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
    Proceed(crate::StateRevision, Option<Box<DurableMachine>>),
    Replay(Box<crate::ExecutionReceipt>),
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn admit_workspace_start(
        &mut self,
        request: &LaunchMachineRequest,
        intent: &LaunchIntent,
        legacy_fingerprint: &crate::RequestFingerprint,
        resolved_at: u64,
        milestones: &mut Vec<Milestone>,
    ) -> Result<LaunchAdmission, ManagedFailure> {
        let Some(stored) = self.load_machine(&request.instance_id)? else {
            return self.admit_launch(request, intent, resolved_at, milestones);
        };
        match stored.machine.phase {
            DurablePhase::Terminal {
                operation: OperationKind::Stop,
                ..
            } => {
                let Some(active) = stored.machine.stopped_source(intent) else {
                    return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
                };
                let allocation = active.launch_receipt.request_fingerprint();
                if allocation != &intent.request_fingerprint && allocation != legacy_fingerprint {
                    return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
                }
                self.admit_restore(request, intent, None, resolved_at, milestones)
            }
            DurablePhase::Launching {
                prior_stop: Some(_),
                ..
            } => self.admit_restore(request, intent, None, resolved_at, milestones),
            _ => self.admit_launch(request, intent, resolved_at, milestones),
        }
    }

    pub(super) fn admit_restore(
        &mut self,
        request: &LaunchMachineRequest,
        intent: &LaunchIntent,
        checkpoint: Option<&crate::OperationId>,
        resolved_at: u64,
        milestones: &mut Vec<Milestone>,
    ) -> Result<LaunchAdmission, ManagedFailure> {
        let stored = self
            .load_machine(&request.instance_id)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        match &stored.machine.phase {
            DurablePhase::Active { active }
                if active.launch_receipt.operation_id() == &request.operation_id
                    && active.launch_receipt.request_fingerprint()
                        == &intent.request_fingerprint =>
            {
                return Ok(LaunchAdmission::Replay(Box::new(
                    active.launch_receipt.clone(),
                )));
            }
            DurablePhase::Launching {
                intent: existing,
                prior_stop: Some(stopped),
            } if existing.as_ref() == intent => {
                // Admission runs with the exclusive restore lease. No earlier request or
                // half-started host can still begin a launch on this Instance.
                let cleanup = self.perform_cleanup(
                    &intent.operation_id,
                    &request.instance_id,
                    CleanupReason::Rollback,
                    milestones,
                );
                if !cleanup.complete
                    || !self.reconcile_checkpoint_launch(intent, request, checkpoint)
                {
                    return Err(ManagedFailure::State(ManagedStateError::RecoveryRequired));
                }
                *milestones = vec![
                    Milestone::new(MilestoneKind::Accepted, 0),
                    Milestone::new(MilestoneKind::WorkloadResolved, resolved_at),
                ];
                return Ok(LaunchAdmission::Proceed(
                    stored.revision,
                    Some(stopped.clone()),
                ));
            }
            _ => {}
        }
        if stored.machine.stopped_source(intent).is_none() {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        let pending = DurableMachine::restoring(
            request.instance_id.clone(),
            intent.clone(),
            stored.machine.clone(),
        );
        let revision = self
            .replace_machine(stored.revision, &pending)
            .map_err(|failure| {
                if is_store_conflict(&failure) {
                    ManagedFailure::State(ManagedStateError::OperationConflict)
                } else {
                    failure
                }
            })?;
        Ok(LaunchAdmission::Proceed(
            revision,
            Some(Box::new(stored.machine)),
        ))
    }

    pub(super) fn admit_launch(
        &mut self,
        request: &LaunchMachineRequest,
        intent: &LaunchIntent,
        resolved_at: u64,
        milestones: &mut Vec<Milestone>,
    ) -> Result<LaunchAdmission, ManagedFailure> {
        let pending = DurableMachine::launching(request.instance_id.clone(), intent.clone());
        match self.create_machine(&pending) {
            Ok(revision) => return Ok(LaunchAdmission::Proceed(revision, None)),
            Err(failure) if is_store_conflict(&failure) => {}
            Err(failure) => return Err(failure),
        }
        let stored = self
            .load_machine(&request.instance_id)?
            .ok_or(ManagedFailure::StateStore(
                crate::StateStoreFailureKind::Conflict,
            ))?;
        match &stored.machine.phase {
            DurablePhase::Launching {
                intent: existing,
                prior_stop: None,
            } if existing.as_ref() == intent => {
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
                Ok(LaunchAdmission::Proceed(revision, None))
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
