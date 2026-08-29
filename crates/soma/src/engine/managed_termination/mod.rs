mod admission;
mod mode;
mod receipt;

use crate::{Backend, Milestone, MilestoneKind, StateStore, TerminalStatus};

use super::{
    DestroyMachineRequest, Engine, MachineDestroy, MachineStop, ManagedFailure, RunFailureKind,
    StopMachineRequest, managed_receipt::operation_failure,
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
        )
        .map(|receipt| MachineDestroy { receipt })
    }

    fn terminate_machine(
        &mut self,
        operation_id: crate::OperationId,
        instance_id: &crate::InstanceId,
        mode: TerminationMode,
    ) -> Result<crate::ExecutionReceipt, ManagedFailure> {
        let admission = match self.admit_termination(&operation_id, instance_id, mode)? {
            TerminationAdmission::Proceed(admission) => admission,
            TerminationAdmission::Replay(receipt) => return Ok(receipt),
        };
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];
        let cleanup = self.perform_cleanup(
            &operation_id,
            instance_id,
            mode.cleanup_reason(),
            &mut milestones,
        );
        let terminal = if cleanup.complete {
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
}
