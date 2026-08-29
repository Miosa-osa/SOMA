use crate::{Backend, StateStore};

use super::{
    super::{Engine, ExecuteMachineRequest, ManagedFailure, ManagedStateError},
    durable_with_phase,
};
use crate::engine::{
    machine_state::{
        ActiveMachine, DurablePhase, MAX_EXECUTION_TOMBSTONES, TerminalBasis, VersionedMachine,
    },
    managed::ReplayEvidence,
};

pub(super) struct ExecutionAdmission {
    pub(super) revision: crate::StateRevision,
    pub(super) active: ActiveMachine,
    pub(super) fingerprint: crate::RequestFingerprint,
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn admit_execution(
        &mut self,
        request: &ExecuteMachineRequest,
    ) -> Result<ExecutionAdmission, ManagedFailure> {
        let stored = self
            .load_machine(&request.instance_id)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        let VersionedMachine { revision, machine } = stored;
        let instance_id = machine.instance_id.clone();
        match machine.phase {
            DurablePhase::Active { active } => {
                ensure_backend(&active, self.backend.kind())?;
                let workload = active.workload().ok_or(ManagedFailure::StateStore(
                    crate::StateStoreFailureKind::Corrupt,
                ))?;
                let fingerprint = crate::fingerprint::execute(
                    workload,
                    &request.instance_id,
                    &request.command,
                    &request.limits,
                );
                if let Some(completed) = active.completed(&request.operation_id) {
                    return if completed.request_fingerprint == fingerprint {
                        Err(ManagedFailure::ReplayUnavailable(
                            ReplayEvidence::from_tombstone(completed),
                        ))
                    } else {
                        Err(ManagedFailure::State(ManagedStateError::OperationConflict))
                    };
                }
                if active.completed_executions.len() >= MAX_EXECUTION_TOMBSTONES {
                    return Err(ManagedFailure::State(
                        ManagedStateError::ReplayCapacityReached,
                    ));
                }
                let executing = durable_with_phase(
                    &request.instance_id,
                    DurablePhase::Executing {
                        active: active.clone(),
                        operation_id: request.operation_id.clone(),
                        request_fingerprint: fingerprint.clone(),
                    },
                );
                let revision = self.replace_machine(revision, &executing)?;
                Ok(ExecutionAdmission {
                    revision,
                    active: *active,
                    fingerprint,
                })
            }
            DurablePhase::Executing {
                active,
                operation_id,
                request_fingerprint,
            } => Err(self.recover_interrupted_execution(
                revision,
                &instance_id,
                *active,
                operation_id,
                request_fingerprint,
            )),
            DurablePhase::Terminal {
                basis,
                operation: crate::OperationKind::Execute,
                operation_id,
                request_fingerprint,
                receipt,
            } if operation_id == request.operation_id => {
                let TerminalBasis::Active { active } = *basis else {
                    return Err(ManagedFailure::StateStore(
                        crate::StateStoreFailureKind::Corrupt,
                    ));
                };
                let workload_fingerprint = active.workload().map(|workload| {
                    crate::fingerprint::execute(
                        workload,
                        &request.instance_id,
                        &request.command,
                        &request.limits,
                    )
                });
                if workload_fingerprint.as_ref() == Some(&request_fingerprint) {
                    Err(ManagedFailure::ReplayUnavailable(
                        ReplayEvidence::from_receipt(*receipt),
                    ))
                } else {
                    Err(ManagedFailure::State(ManagedStateError::OperationConflict))
                }
            }
            DurablePhase::Terminal { .. } => {
                Err(ManagedFailure::State(ManagedStateError::MachineStopped))
            }
            DurablePhase::Launching { .. } | DurablePhase::Terminating { .. } => {
                Err(ManagedFailure::State(ManagedStateError::RecoveryRequired))
            }
        }
    }
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
