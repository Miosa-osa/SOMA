mod admission;
mod failure;
mod receipt;

use crate::{
    Backend, CleanupEvidence, Milestone, MilestoneKind, StateStore, TerminalStatus,
    WorkloadEvidence,
};

use super::{
    Engine, FailurePhase, LaunchMachineRequest, MachineLaunch, ManagedFailure, ManagedStateError,
    RunFailureKind,
    machine_state::{DurableMachine, LaunchIntent},
    managed_receipt::{ManagedLaunch, operation_failure},
    run_evidence::{append_failure, append_launch, append_milestone, validate_launch},
};

use self::{
    admission::LaunchAdmission,
    receipt::{launch_receipt, store_operation_failure},
};

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
                    &request,
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
        );
        let intent = LaunchIntent {
            operation_id: request.operation_id.clone(),
            machine_name: request.machine_name.clone(),
            workload: workload.clone(),
            requested_shape: request.shape.clone(),
            backend: self.backend.kind(),
            request_fingerprint: fingerprint.clone(),
        };
        let revision = match self.admit_launch(&request, &intent, resolved_at, &mut milestones)? {
            LaunchAdmission::Proceed(revision) => revision,
            LaunchAdmission::Replay(receipt) => return Ok(MachineLaunch { receipt: *receipt }),
        };

        let observation = self.backend.launch(crate::LaunchRequest::new(
            &request.operation_id,
            &request.instance_id,
            &workload,
            &prepared,
            &request.shape,
        ));
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
                    &request,
                    intent,
                    revision,
                    fingerprint,
                    workload,
                    milestones,
                    kind,
                ));
            }
        };
        let Some(launch) = launch else {
            return Err(self.fail_launch(
                &request,
                intent,
                revision,
                fingerprint,
                workload,
                milestones,
                RunFailureKind::ObservationMismatch,
            ));
        };
        append_launch(&mut milestones, launch.times);
        let receipt = launch_receipt(
            &request,
            fingerprint,
            WorkloadEvidence::Resolved { identity: workload },
            launch.backend,
            ManagedLaunch::Fresh(launch),
            milestones,
            CleanupEvidence::not_owned(),
            TerminalStatus::Ready,
        );
        let active = DurableMachine::active(request.instance_id.clone(), receipt.clone());
        if let Err(failure) = self.replace_machine(revision, &active) {
            return Err(store_operation_failure(&failure, receipt));
        }
        Ok(MachineLaunch { receipt })
    }
}
