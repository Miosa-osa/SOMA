//! Launch a managed machine: admit it, launch it on the backend, and record its receipt.

use super::{Backend, CleanupEvidence, DurableMachine, DurablePhase, Engine, FailurePhase, LaunchAdmission, LaunchIntent, LaunchMachineRequest, LockedLaunch, MachineLaunch, ManagedFailure, ManagedLaunch, ManagedStateError, Milestone, MilestoneKind, RunFailureKind, StateStore, TerminalStatus, WorkloadEvidence, admission, append_failure, append_launch, append_milestone, failure, fork, launch_receipt, operation_failure, receipt, store_operation_failure, validate_launch};

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn launch_managed(
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
                checkpoint,
                fork_source: None,
                ..
            }) => self.admit_restore(
                request,
                &intent,
                Some(checkpoint),
                resolved_at,
                &mut milestones,
            )?,
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
        .with_windowed(request.windowed)
        .with_resume(request.resume.as_deref());
        let recovery_checkpoint = match &locked {
            Some(LockedLaunch::Checkpoint {
                checkpoint,
                fork_source: None,
                ..
            }) => Some(*checkpoint),
            _ => None,
        };
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
                    recovery_checkpoint,
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
                recovery_checkpoint,
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
