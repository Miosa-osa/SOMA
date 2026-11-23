use crate::{
    Backend, InstanceId, OperationId, OperationKind, StateStore, TerminalStatus, WorkloadIdentity,
};

use super::{Engine, LaunchMachineRequest, MachineLaunch, ManagedFailure, ManagedStateError};
use crate::engine::machine_state::{DurablePhase, TerminalBasis};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Launches a distinct managed Instance using the authorized source's checkpoint.
    ///
    /// The trusted caller supplies the destination name with the authenticated tenant label.
    ///
    /// # Errors
    /// Returns a typed refusal when the source, destination ownership, workload, shape, or
    /// checkpoint is incompatible, or the backend cannot prove recovery.
    #[allow(clippy::needless_pass_by_value)]
    pub fn fork_machine(
        &mut self,
        request: LaunchMachineRequest,
        source: &InstanceId,
        checkpoint: &OperationId,
    ) -> Result<MachineLaunch, ManagedFailure> {
        if source == &request.instance_id || request.machine_name.is_none() {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        self.launch_from_checkpoint(&request, checkpoint, Some(source))
    }

    pub(super) fn validate_fork_source(
        &mut self,
        source: &InstanceId,
        request: &LaunchMachineRequest,
        workload: &WorkloadIdentity,
    ) -> Result<(), ManagedFailure> {
        let missing = ManagedFailure::State(ManagedStateError::MachineNotFound);
        let stored = self
            .load_machine(source)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        let active = match &stored.machine.phase {
            DurablePhase::Active { active } | DurablePhase::Executing { active, .. } => active,
            DurablePhase::Terminal {
                operation: OperationKind::Stop,
                basis,
                receipt,
                ..
            } if receipt.terminal_status() == &TerminalStatus::Stopped => {
                let TerminalBasis::Active { active } = basis.as_ref() else {
                    return Err(missing);
                };
                active
            }
            _ => return Err(missing),
        };
        if active.machine_name() != request.machine_name.as_ref() {
            return Err(missing);
        }
        if active.workload() != Some(workload)
            || active.shape() != &request.shape
            || active.backend() != self.backend.kind()
        {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        Ok(())
    }
}
