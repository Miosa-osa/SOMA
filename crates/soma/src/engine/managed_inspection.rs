use crate::{
    Backend, CleanupEvidence, MeasurementClass, Milestone, MilestoneKind, Observation,
    ObservationUnavailable, OperationKind, SandboxLiveness, StateStore, TerminalStatus,
    WorkloadEvidence,
};

use super::{
    Engine, FailurePhase, InspectMachineRequest, MachineInspection, ManagedFailure,
    ManagedStateError, RunFailureKind,
    machine_state::{ActiveMachine, DurablePhase, TerminalBasis},
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
            DurablePhase::Launching { .. }
            | DurablePhase::Executing { .. }
            | DurablePhase::Terminating { .. } => {
                return Err(ManagedFailure::State(ManagedStateError::RecoveryRequired));
            }
            ended => return self.ended(&request, ended),
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
            // The machine is gone. A caller that restarts a stopped sandbox has to be told so,
            // not handed a backend failure it cannot act on. Nothing is written: the record is
            // released by the stop or create the caller sends next.
            Err(_) if self.backend.liveness(&request.instance_id) == SandboxLiveness::Absent => {
                return Ok(lost_machine(&request, &active, fingerprint));
            }
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
        Ok(MachineInspection {
            state,
            receipt,
            machine_lost: false,
        })
    }
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// What reading an Instance whose record ended answers.
    fn ended(
        &self,
        request: &InspectMachineRequest,
        phase: DurablePhase,
    ) -> Result<MachineInspection, ManagedFailure> {
        match phase {
            // A command took the machine down, so it reads as a machine that was lost.
            DurablePhase::Terminal {
                basis,
                operation: OperationKind::Execute,
                ..
            } => self.ended_machine(request, *basis, true),
            // A stopped sandbox reads as stopped: the platform reads that state to show it and
            // to start it again.
            DurablePhase::Terminal {
                basis,
                operation: OperationKind::Stop,
                ..
            } => self.ended_machine(request, *basis, false),
            // A create that failed made no sandbox, and a destroyed one is gone. What the record
            // still holds is the answer an exact repeat replays, so reading the identity says
            // there is no sandbox rather than that its state refused the read.
            DurablePhase::Terminal {
                operation: OperationKind::Launch | OperationKind::Destroy,
                ..
            } => Err(ManagedFailure::State(ManagedStateError::MachineNotFound)),
            _ => Err(ManagedFailure::State(ManagedStateError::MachineStopped)),
        }
    }

    /// A stopped answer for a record that ended with its machine: lost when a command took the
    /// machine down, not lost when the sandbox was stopped.
    fn ended_machine(
        &self,
        request: &InspectMachineRequest,
        basis: TerminalBasis,
        machine_lost: bool,
    ) -> Result<MachineInspection, ManagedFailure> {
        let TerminalBasis::Active { active } = basis else {
            return Err(ManagedFailure::State(ManagedStateError::MachineStopped));
        };
        ensure_backend(&active, self.backend.kind())?;
        let workload = active.workload().ok_or(ManagedFailure::StateStore(
            crate::StateStoreFailureKind::Corrupt,
        ))?;
        let fingerprint = crate::fingerprint::inspect(workload, &request.instance_id);
        let mut inspection = lost_machine(request, &active, fingerprint);
        inspection.machine_lost = machine_lost;
        Ok(inspection)
    }
}

/// What an inspection answers for a machine whose host is gone: stopped, and nothing written.
fn lost_machine(
    request: &InspectMachineRequest,
    active: &ActiveMachine,
    fingerprint: crate::RequestFingerprint,
) -> MachineInspection {
    let state = crate::MachineState::Stopped;
    let milestones = vec![
        Milestone::new(MilestoneKind::Accepted, 0),
        Milestone::new(MilestoneKind::Inspected, 0),
    ];
    let receipt = inspection_receipt(
        request,
        active,
        fingerprint,
        milestones,
        TerminalStatus::Inspected { state },
        None,
    );
    MachineInspection {
        state,
        receipt,
        machine_lost: true,
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
