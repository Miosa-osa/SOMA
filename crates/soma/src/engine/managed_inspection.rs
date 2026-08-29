use crate::{
    Backend, CleanupEvidence, MeasurementClass, Milestone, MilestoneKind, Observation,
    ObservationUnavailable, OperationKind, StateStore, TerminalStatus, WorkloadEvidence,
};

use super::{
    Engine, FailurePhase, InspectMachineRequest, MachineInspection, ManagedFailure,
    ManagedStateError, RunFailureKind,
    machine_state::{ActiveMachine, DurablePhase},
    managed_receipt::{ManagedLaunch, ManagedReceipt, managed_receipt, operation_failure},
    run_evidence::{append_failure, append_milestone},
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Inspects one exact managed Instance and returns bounded typed state evidence.
    ///
    /// # Errors
    ///
    /// Returns typed state, store, or evidence-carrying backend failures.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the use-case boundary takes ownership of its immutable request"
    )]
    pub fn inspect_machine(
        &mut self,
        request: InspectMachineRequest,
    ) -> Result<MachineInspection, ManagedFailure> {
        let stored = self
            .load_machine(&request.instance_id)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        let revision = stored.revision;
        let active = match stored.machine.phase {
            DurablePhase::Active { active } => active,
            DurablePhase::Terminal { .. } => {
                return Err(ManagedFailure::State(ManagedStateError::MachineStopped));
            }
            DurablePhase::Launching { .. }
            | DurablePhase::Executing { .. }
            | DurablePhase::Terminating { .. } => {
                return Err(ManagedFailure::State(ManagedStateError::RecoveryRequired));
            }
        };
        ensure_backend(&active, self.backend.kind())?;
        let workload = active
            .workload()
            .ok_or(ManagedFailure::StateStore(
                crate::StateStoreFailureKind::Corrupt,
            ))?
            .clone();
        let fingerprint = crate::fingerprint::inspect(&workload, &request.instance_id);
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];
        let observation = match self.backend.inspect(crate::InspectionRequest::new(
            &request.operation_id,
            &request.instance_id,
            &workload,
            active.shape(),
        )) {
            Ok(observation) => observation,
            Err(failure) => {
                let kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: FailurePhase::Inspect,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                let receipt = inspection_receipt(
                    &request,
                    &active,
                    fingerprint,
                    milestones,
                    TerminalStatus::Failed,
                    None,
                );
                return Err(ManagedFailure::operation(operation_failure(
                    kind, receipt, None,
                )));
            }
        };
        let crate::backend::InspectionObservationParts {
            operation_id: operation,
            instance_id: instance,
            workload: observed_workload,
            backend,
            state,
            effective_network,
            observed_at_ns: observed_at,
        } = observation.into_parts();
        if operation != request.operation_id
            || instance != request.instance_id
            || observed_workload != workload
            || backend != active.backend()
            || !effective_network.matches_request(active.shape().capabilities().network_policy())
            || !append_milestone(&mut milestones, MilestoneKind::Inspected, observed_at)
        {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        let current = self
            .load_machine(&request.instance_id)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        if current.revision != revision {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        let receipt = inspection_receipt(
            &request,
            &active,
            fingerprint,
            milestones,
            TerminalStatus::Inspected { state },
            Some(effective_network),
        );
        Ok(MachineInspection { state, receipt })
    }
}

fn inspection_receipt(
    request: &InspectMachineRequest,
    active: &ActiveMachine,
    fingerprint: crate::RequestFingerprint,
    milestones: Vec<Milestone>,
    terminal: TerminalStatus,
    effective_network: Option<crate::EffectiveNetwork>,
) -> crate::ExecutionReceipt {
    managed_receipt(ManagedReceipt {
        operation: OperationKind::Inspect,
        operation_id: request.operation_id.clone(),
        instance_id: request.instance_id.clone(),
        machine_name: active.machine_name().cloned(),
        fingerprint,
        workload: WorkloadEvidence::Resolved {
            identity: active
                .workload()
                .expect("validated active state has resolved workload")
                .clone(),
        },
        backend: active.backend(),
        launch: effective_network.map_or_else(
            || ManagedLaunch::Stored(Box::new(active.launch_receipt.clone())),
            |effective_network| ManagedLaunch::Inspected {
                receipt: Box::new(active.launch_receipt.clone()),
                effective_network,
            },
        ),
        shape: active.shape().clone(),
        milestones,
        terminal,
        output: Observation::Unavailable(ObservationUnavailable::NotReached),
        cleanup: CleanupEvidence::not_owned(),
        measurement: MeasurementClass::FacadeManagedInspect,
    })
}

fn ensure_backend(
    active: &ActiveMachine,
    backend: crate::BackendKind,
) -> Result<(), ManagedFailure> {
    if active.backend() == backend {
        Ok(())
    } else {
        Err(ManagedFailure::StateStore(
            crate::StateStoreFailureKind::Corrupt,
        ))
    }
}
