use crate::{Backend, StateStore};

use crate::engine::{
    Engine, ManagedFailure, ManagedStateError,
    machine_state::{ActiveMachine, DurableMachine, DurablePhase, LaunchIntent, TerminalBasis},
};

use super::mode::TerminationMode;

pub(super) enum TerminationAdmission {
    Proceed(TerminationContext),
    Replay(crate::ExecutionReceipt),
    /// A destroy of an Instance that already ended (a failed command, a stop) with its cleanup
    /// complete. The machine owes the host nothing, so the destroy records itself without asking
    /// the backend for anything.
    AlreadyEnded(TerminationContext),
    /// A destroy of an Instance whose launch never finished: one left launching (the host restarted
    /// under it), or one whose launch failed. `cleaned` says the failed launch already released
    /// the host; a launch left launching is rolled back first (SOMA#220).
    NeverLaunched {
        revision: crate::StateRevision,
        intent: Box<LaunchIntent>,
        cleaned: bool,
    },
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
            // A stop of a stopped Instance is answered with the stop that stopped it (SOMA#220).
            DurablePhase::Terminal {
                operation: crate::OperationKind::Stop,
                receipt,
                ..
            } if matches!(mode, TerminationMode::Graceful) => {
                Ok(TerminationAdmission::Replay(*receipt))
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
            DurablePhase::Launching { intent, .. } => never_launched(mode, revision, intent),
            DurablePhase::Terminal { basis, receipt, .. }
                if matches!(mode, TerminationMode::Forced) && receipt.cleanup().is_complete() =>
            {
                self.admit_ended_destroy(revision, instance_id, mode, *basis)
            }
            DurablePhase::Terminal { .. } | DurablePhase::Recovery { .. } => {
                Err(ManagedFailure::State(ManagedStateError::MachineStopped))
            }
            DurablePhase::Terminating { .. } => {
                Err(ManagedFailure::State(ManagedStateError::OperationConflict))
            }
        }
    }
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Admits the destroy of an Instance whose end already cleaned it up.
    ///
    /// Before this, a sandbox whose command took the machine down answered every destroy with
    /// 409 for good, so a caller could never let go of it.
    fn admit_ended_destroy(
        &self,
        revision: crate::StateRevision,
        instance_id: &crate::InstanceId,
        mode: TerminationMode,
        basis: TerminalBasis,
    ) -> Result<TerminationAdmission, ManagedFailure> {
        let active = match basis {
            TerminalBasis::Active { active } => active,
            TerminalBasis::Launch { intent } => {
                return Ok(TerminationAdmission::NeverLaunched {
                    revision,
                    intent,
                    cleaned: true,
                });
            }
        };
        ensure_backend(&active, self.backend.kind())?;
        let workload = active.workload().ok_or(ManagedFailure::StateStore(
            crate::StateStoreFailureKind::Corrupt,
        ))?;
        let fingerprint = mode.fingerprint(workload, instance_id);
        Ok(TerminationAdmission::AlreadyEnded(TerminationContext {
            revision,
            active: *active,
            fingerprint,
        }))
    }
}

/// A destroy of a launch left launching rolls it back; any other request waits for recovery.
fn never_launched(
    mode: TerminationMode,
    revision: crate::StateRevision,
    intent: Box<LaunchIntent>,
) -> Result<TerminationAdmission, ManagedFailure> {
    if matches!(mode, TerminationMode::Forced) {
        Ok(TerminationAdmission::NeverLaunched {
            revision,
            intent,
            cleaned: false,
        })
    } else {
        Err(ManagedFailure::State(ManagedStateError::RecoveryRequired))
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
