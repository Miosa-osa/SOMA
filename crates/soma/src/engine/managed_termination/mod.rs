mod admission;
mod mode;
mod receipt;

use std::path::Path;

use crate::{
    Backend, BackendFailureKind, Milestone, MilestoneKind, StateStore, SuspendObservation,
    SuspendRequest, TerminalStatus,
};

use super::{
    DestroyMachineRequest, Engine, MachineDestroy, MachineStop, MachineSuspend, ManagedFailure,
    RunFailureKind, StopMachineRequest, SuspendMachineRequest,
    machine_state::{DurableMachine, DurablePhase},
    managed_receipt::operation_failure,
};

use self::{
    admission::TerminationAdmission,
    mode::TerminationMode,
    receipt::{durable_with_terminal, store_operation_failure, termination_receipt},
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Gracefully stops an exact managed Instance and retains an exact replay receipt.
    ///
    /// # Errors
    ///
    /// Returns typed state, store, or evidence-carrying cleanup failures.
    pub fn stop_machine(
        &mut self,
        request: StopMachineRequest,
    ) -> Result<MachineStop, ManagedFailure> {
        self.terminate_machine(
            request.operation_id,
            &request.instance_id,
            TerminationMode::Graceful,
            None,
        )
        .map(|receipt| MachineStop { receipt })
    }

    /// Force-destroys an exact managed Instance and retains an exact replay receipt.
    ///
    /// # Errors
    ///
    /// Returns typed state, store, or evidence-carrying cleanup failures.
    pub fn destroy_machine(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<MachineDestroy, ManagedFailure> {
        self.terminate_machine(
            request.operation_id,
            &request.instance_id,
            TerminationMode::Forced,
            None,
        )
        .map(|receipt| MachineDestroy { receipt })
    }

    /// Reads an exact managed Instance into a Suspension's staging directory and ends it, with an
    /// exact replay receipt (ADR 0034). A resume is a new Instance.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedFailure::Backend`] with [`BackendFailureKind::Unsupported`] when the
    /// Instance cannot be suspended, in which case it keeps running untouched, and otherwise the
    /// typed state, store, or evidence-carrying failures a termination returns.
    pub fn suspend_machine(
        &mut self,
        request: &SuspendMachineRequest,
    ) -> Result<MachineSuspend, ManagedFailure> {
        self.terminate_machine(
            request.operation_id.clone(),
            &request.instance_id,
            TerminationMode::Suspend,
            Some(&request.staging),
        )
        .map(|receipt| MachineSuspend { receipt })
    }

    fn terminate_machine(
        &mut self,
        operation_id: crate::OperationId,
        instance_id: &crate::InstanceId,
        mode: TerminationMode,
        staging: Option<&Path>,
    ) -> Result<crate::ExecutionReceipt, ManagedFailure> {
        let admission = match self.admit_termination(&operation_id, instance_id, mode)? {
            TerminationAdmission::Proceed(admission) => admission,
            TerminationAdmission::Replay(receipt) => return Ok(receipt),
        };
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];
        // A suspension reads the machine before anything is released. A refusal means nothing
        // happened, so the record goes back to active and the Instance keeps running; a failure
        // after the guest acknowledged may have lost the machine, so it is cleaned up and the
        // receipt says Failed rather than Suspended.
        let mut suspend_failed = false;
        if let (TerminationMode::Suspend, Some(staging)) = (mode, staging) {
            let request = SuspendRequest::new(&operation_id, instance_id, staging);
            match self.backend.suspend(request) {
                Ok(SuspendObservation::Suspended) => {}
                Ok(SuspendObservation::Refused) => {
                    self.back_to_active(instance_id, &admission);
                    return Err(ManagedFailure::Backend(BackendFailureKind::Unsupported));
                }
                Err(_) => suspend_failed = true,
            }
        }
        let cleanup = self.perform_cleanup(
            &operation_id,
            instance_id,
            mode.cleanup_reason(),
            &mut milestones,
        );
        let terminal = if cleanup.complete && !suspend_failed {
            mode.terminal_status()
        } else {
            TerminalStatus::Failed
        };
        let receipt = termination_receipt(
            &operation_id,
            instance_id,
            &admission.active,
            admission.fingerprint.clone(),
            mode,
            milestones,
            terminal,
            cleanup.evidence,
        );
        if !cleanup.complete {
            // #173: the machine is still owned, so the record goes back to active and a later destroy
            // retries it. Left terminating, every later request (a new operation id) answered 409 for good.
            self.back_to_active(instance_id, &admission);
            return Err(ManagedFailure::operation(operation_failure(
                cleanup
                    .failure_kind
                    .unwrap_or(RunFailureKind::CleanupIncomplete),
                receipt,
                None,
            )));
        }
        let terminal_state = durable_with_terminal(
            instance_id,
            admission.active,
            mode.operation(),
            operation_id,
            admission.fingerprint,
            receipt.clone(),
        );
        if let Err(failure) = self.replace_machine(admission.revision, &terminal_state) {
            return Err(store_operation_failure(&failure, receipt));
        }
        Ok(receipt)
    }

    /// Puts an admitted termination's record back to active, for a machine still owned.
    ///
    /// The caller's failure is the answer; a store refusal here leaves the record as it was.
    fn back_to_active(
        &mut self,
        instance_id: &crate::InstanceId,
        admission: &admission::TerminationContext,
    ) {
        let mut back =
            DurableMachine::active(instance_id.clone(), admission.active.launch_receipt.clone());
        back.phase = DurablePhase::Active {
            active: Box::new(admission.active.clone()),
        };
        let _ = self.replace_machine(admission.revision, &back);
    }
}
