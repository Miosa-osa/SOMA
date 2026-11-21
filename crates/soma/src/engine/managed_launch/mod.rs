mod admission;
mod failure;
mod fork;
mod receipt;

use crate::{
    Backend, CleanupEvidence, Milestone, MilestoneKind, StateStore, TerminalStatus,
    WorkloadEvidence,
};

use super::{
    Engine, FailurePhase, LaunchMachineRequest, MachineLaunch, ManagedFailure, ManagedStateError,
    RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, LaunchIntent},
    managed_receipt::{ManagedLaunch, operation_failure},
    run_evidence::{append_failure, append_launch, append_milestone, validate_launch},
};

use self::{
    admission::LaunchAdmission,
    receipt::{launch_receipt, store_operation_failure},
};

#[derive(Clone, Copy)]
enum LockedLaunch<'a> {
    Workspace(&'a (dyn std::any::Any + Send)),
    Checkpoint {
        checkpoint: &'a crate::OperationId,
        fork_source: Option<&'a crate::InstanceId>,
        lease: &'a (dyn std::any::Any + Send),
    },
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Launches a managed Machine after durably recording its intent.
    ///
    /// # Errors
    ///
    /// Returns typed state, store, replay, or operation evidence on any failed transition.
    #[allow(
        clippy::needless_pass_by_value,
        clippy::too_many_lines,
        reason = "the owned request drives one explicit write-ahead launch transaction"
    )]
    pub fn launch_machine(
        &mut self,
        request: LaunchMachineRequest,
    ) -> Result<MachineLaunch, ManagedFailure> {
        if request.volume.is_some() {
            let lease = self
                .backend
                .acquire_restore(&request.instance_id)
                .map_err(|failure| ManagedFailure::Backend(failure.kind()))?;
            return self.launch_managed(&request, Some(LockedLaunch::Workspace(lease.as_ref())));
        }
        self.launch_managed(&request, None)
    }

    /// Returns the recorded name used to check restore ownership after a runner restart.
    /// # Errors
    /// Returns the durable-store failure if ownership cannot be read.
    pub fn restore_owner(
        &mut self,
        instance: &crate::InstanceId,
    ) -> Result<Option<crate::MachineName>, ManagedFailure> {
        use crate::{OperationKind, engine::machine_state::TerminalBasis};

        let Some(stored) = self.load_machine(instance)? else {
            return Ok(None);
        };
        let name = match stored.machine.phase {
            DurablePhase::Terminal {
                operation: OperationKind::Stop,
                basis,
                ..
            } => match *basis {
                TerminalBasis::Active { active } => active.machine_name().cloned(),
                TerminalBasis::Launch { .. } => None,
            },
            DurablePhase::Launching {
                intent,
                prior_stop: Some(_),
            } => intent.machine_name.clone(),
            DurablePhase::Active { active } => active.machine_name().cloned(),
            _ => None,
        };
        Ok(name)
    }

    /// Restores a checkpoint created for the same stopped Instance.
    /// # Errors
    /// Returns a typed failure if the stopped record, admitted Generation, or checkpoint cannot
    /// be confirmed, or the backend cannot prove readiness.
    #[allow(clippy::needless_pass_by_value)]
    pub fn restore_machine(
        &mut self,
        request: LaunchMachineRequest,
        checkpoint: &crate::OperationId,
    ) -> Result<MachineLaunch, ManagedFailure> {
        self.launch_from_checkpoint(&request, checkpoint, None)
    }

    fn launch_from_checkpoint(
        &mut self,
        request: &LaunchMachineRequest,
        checkpoint: &crate::OperationId,
        fork_source: Option<&crate::InstanceId>,
    ) -> Result<MachineLaunch, ManagedFailure> {
        if request.operation_id == *checkpoint
            || request.volume.is_some()
            || request.windowed
            || *request.shape.capabilities().network_policy() != crate::NetworkPolicy::isolated()
        {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        let lease = self
            .backend
            .acquire_restore(&request.instance_id)
            .map_err(|failure| ManagedFailure::Backend(failure.kind()))?;
        self.launch_managed(
            request,
            Some(LockedLaunch::Checkpoint {
                checkpoint,
                fork_source,
                lease: lease.as_ref(),
            }),
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "fresh launch and restore share the write-ahead transaction"
    )]
    fn launch_managed(
        &mut self,
        request: &LaunchMachineRequest,
        locked: Option<LockedLaunch<'_>>,
    ) -> Result<MachineLaunch, ManagedFailure> {
        let source_fingerprint = crate::fingerprint::source(&request.image);
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];
        let resolution = match self.backend.resolve(crate::ResolutionRequest::new(
            &request.operation_id,
            &request.image,
            &source_fingerprint,
        )) {
            Ok(resolution) => resolution,
            Err(failure) => {
                let kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: FailurePhase::Resolution,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                let receipt = launch_receipt(
                    request,
                    source_fingerprint.clone(),
                    WorkloadEvidence::Unresolved { source_fingerprint },
                    self.backend.kind(),
                    ManagedLaunch::NotReached,
                    milestones,
                    CleanupEvidence::not_owned(),
                    TerminalStatus::Failed,
                );
                return Err(ManagedFailure::operation(operation_failure(
                    kind, receipt, None,
                )));
            }
        };
        let (observed_operation, observed_source, workload, prepared, resolved_at) =
            resolution.into_parts();
        if observed_operation != request.operation_id
            || observed_source != source_fingerprint
            || !append_milestone(
                &mut milestones,
                MilestoneKind::WorkloadResolved,
                resolved_at,
            )
        {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        let fingerprint = crate::fingerprint::launch(
            &workload,
            &request.instance_id,
            request.machine_name.as_ref(),
            &request.shape,
            request.windowed,
        );
        let unbound_fingerprint = fingerprint;
        let fingerprint = match &locked {
            Some(LockedLaunch::Checkpoint {
                checkpoint,
                fork_source: Some(source),
                ..
            }) => {
                self.validate_fork_source(source, request, &workload)?;
                crate::fingerprint::fork(&unbound_fingerprint, checkpoint, source)
            }
            Some(LockedLaunch::Checkpoint {
                checkpoint,
                fork_source: None,
                ..
            }) => crate::fingerprint::restore(&unbound_fingerprint, checkpoint),
            Some(LockedLaunch::Workspace(_)) => {
                let volume = request
                    .volume
                    .as_ref()
                    .ok_or(ManagedFailure::State(ManagedStateError::OperationConflict))?;
                crate::fingerprint::workspace(&unbound_fingerprint, &volume.id)
            }
            None => unbound_fingerprint.clone(),
        };
        let intent = LaunchIntent {
            operation_id: request.operation_id.clone(),
            machine_name: request.machine_name.clone(),
            workload: workload.clone(),
            requested_shape: request.shape.clone(),
            backend: self.backend.kind(),
            request_fingerprint: fingerprint.clone(),
        };
        let admission = match &locked {
            Some(LockedLaunch::Checkpoint {
                fork_source: None, ..
            }) => self.admit_restore(request, &intent, resolved_at, &mut milestones)?,
            Some(LockedLaunch::Workspace(_)) => self.admit_workspace_start(
                request,
                &intent,
                &unbound_fingerprint,
                resolved_at,
                &mut milestones,
            )?,
            _ => self.admit_launch(request, &intent, resolved_at, &mut milestones)?,
        };
        let (revision, prior_stop) = match admission {
            LaunchAdmission::Proceed(revision, prior_stop) => (revision, prior_stop),
            LaunchAdmission::Replay(receipt) => return Ok(MachineLaunch { receipt: *receipt }),
        };

        let backend_request = crate::LaunchRequest::new(
            &request.operation_id,
            &request.instance_id,
            &workload,
            &prepared,
            &request.shape,
        )
        // The volume the host resolved for this Instance: a machine whose Generation
        // declares one refuses to run without it.
        .with_volume(request.volume.as_ref())
        .with_windowed(request.windowed);
        let observation = match locked {
            Some(LockedLaunch::Checkpoint {
                checkpoint,
                fork_source,
                lease,
            }) => self
                .backend
                .launch_checkpoint(backend_request, checkpoint, fork_source, lease),
            Some(LockedLaunch::Workspace(lease)) => {
                self.backend.launch_workspace(backend_request, lease)
            }
            None => self.backend.launch(backend_request),
        };
        let launch = match observation {
            Ok(observation) => validate_launch(
                observation,
                &request.operation_id,
                &request.instance_id,
                &workload,
                self.backend.kind(),
                &request.shape,
                resolved_at,
            ),
            Err(failure) => {
                let kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: FailurePhase::Launch,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                return Err(self.fail_launch(
                    request,
                    intent,
                    revision,
                    prior_stop,
                    fingerprint,
                    workload,
                    milestones,
                    kind,
                ));
            }
        };
        let Some(launch) = launch else {
            return Err(self.fail_launch(
                request,
                intent,
                revision,
                prior_stop,
                fingerprint,
                workload,
                milestones,
                RunFailureKind::ObservationMismatch,
            ));
        };
        append_launch(&mut milestones, launch.times);
        let receipt = launch_receipt(
            request,
            fingerprint,
            WorkloadEvidence::Resolved { identity: workload },
            launch.backend,
            ManagedLaunch::Fresh(launch),
            milestones,
            CleanupEvidence::not_owned(),
            TerminalStatus::Ready,
        );
        let mut active = DurableMachine::active(request.instance_id.clone(), receipt.clone());
        if let (Some(previous), DurablePhase::Active { active }) = (prior_stop, &mut active.phase)
            && let Some(previous_active) = previous.stopped_source(&intent)
        {
            active
                .completed_executions
                .clone_from(&previous_active.completed_executions);
        }
        if let Err(failure) = self.replace_machine(revision, &active) {
            return Err(store_operation_failure(&failure, receipt));
        }
        Ok(MachineLaunch { receipt })
    }
}
