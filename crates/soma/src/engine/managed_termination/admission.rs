use crate::{Backend, StateStore};

use crate::engine::{
    Engine, ManagedFailure, ManagedStateError,
    machine_state::{ActiveMachine, DurableMachine, DurablePhase},
};

use super::mode::TerminationMode;

pub(super) enum TerminationAdmission {
    Proceed(TerminationContext),
    Replay(crate::ExecutionReceipt),
}

pub(super) struct TerminationContext {
    pub(super) revision: crate::StateRevision,
    pub(super) active: ActiveMachine,
    pub(super) fingerprint: crate::RequestFingerprint,
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    pub(super) fn admit_termination(
        &mut self,
        operation_id: &crate::OperationId,
        instance_id: &crate::InstanceId,
        mode: TerminationMode,
    ) -> Result<TerminationAdmission, ManagedFailure> {
        let stored = self
            .load_machine(instance_id)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        let revision = stored.revision;
        match stored.machine.phase {
            DurablePhase::Active { active } => {
                ensure_backend(&active, self.backend.kind())?;
                let workload = active.workload().ok_or(ManagedFailure::StateStore(
                    crate::StateStoreFailureKind::Corrupt,
                ))?;
                let fingerprint = mode.fingerprint(workload, instance_id);
                let terminating = durable_with_phase(
                    instance_id,
                    DurablePhase::Terminating {
                        active: active.clone(),
                        operation: mode.operation(),
                        operation_id: operation_id.clone(),
                        request_fingerprint: fingerprint.clone(),
                    },
                );
                let revision = self.replace_machine(revision, &terminating)?;
                Ok(TerminationAdmission::Proceed(TerminationContext {
                    revision,
                    active: *active,
                    fingerprint,
                }))
            }
            DurablePhase::Terminating {
                active,
                operation,
                operation_id: existing_operation,
                request_fingerprint,
            } if operation == mode.operation() && existing_operation == *operation_id => {
                let expected = mode.fingerprint(
                    active.workload().ok_or(ManagedFailure::StateStore(
                        crate::StateStoreFailureKind::Corrupt,
                    ))?,
                    instance_id,
                );
                if expected != request_fingerprint {
                    return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
                }
                Ok(TerminationAdmission::Proceed(TerminationContext {
                    revision,
                    active: *active,
                    fingerprint: request_fingerprint,
                }))
            }
            DurablePhase::Terminal {
                operation,
                operation_id: existing_operation,
                request_fingerprint,
                receipt,
                ..
            } if operation == mode.operation() && existing_operation == *operation_id => {
                let expected = receipt.request_fingerprint();
                if expected == &request_fingerprint {
                    Ok(TerminationAdmission::Replay(*receipt))
                } else {
                    Err(ManagedFailure::State(ManagedStateError::OperationConflict))
                }
            }
            DurablePhase::Terminal {
                operation_id: existing_operation,
                ..
            } if existing_operation == *operation_id => {
                Err(ManagedFailure::State(ManagedStateError::OperationConflict))
            }
            DurablePhase::Executing {
                active,
                operation_id,
                request_fingerprint,
            } => Err(self.recover_interrupted_execution(
                revision,
                instance_id,
                *active,
                operation_id,
                request_fingerprint,
            )),
            DurablePhase::Launching { .. } => {
                Err(ManagedFailure::State(ManagedStateError::RecoveryRequired))
            }
            DurablePhase::Terminal { .. } => {
                Err(ManagedFailure::State(ManagedStateError::MachineStopped))
            }
            DurablePhase::Terminating { .. } => {
                Err(ManagedFailure::State(ManagedStateError::OperationConflict))
            }
        }
    }
}

fn durable_with_phase(instance_id: &crate::InstanceId, phase: DurablePhase) -> DurableMachine {
    let launch = match &phase {
        DurablePhase::Terminating { active, .. } => active.launch_receipt.clone(),
        _ => unreachable!(),
    };
    let mut machine = DurableMachine::active(instance_id.clone(), launch);
    machine.phase = phase;
    machine
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
