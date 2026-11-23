mod admission;
mod failure;
mod fork;
mod managed;
mod receipt;
mod recovery;

use crate::{
    Backend, CleanupEvidence, Milestone, MilestoneKind, StateStore, TerminalStatus,
    WorkloadEvidence,
};

use super::{
    Engine, FailurePhase, LaunchMachineRequest, MachineLaunch, ManagedFailure, ManagedStateError,
    RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, LaunchIntent},
    managed_receipt::{ManagedLaunch, operation_failure},
    run_evidence::{append_failure, append_launch, append_milestone, validate_launch},
};

use self::{
    admission::LaunchAdmission,
    receipt::{launch_receipt, store_operation_failure},
};

#[derive(Clone, Copy)]
enum LockedLaunch<'a> {
    Workspace(&'a (dyn std::any::Any + Send)),
    Checkpoint {
        checkpoint: &'a crate::OperationId,
        fork_source: Option<&'a crate::InstanceId>,
        lease: &'a (dyn std::any::Any + Send),
    },
}

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
        if request.volume.is_some() {
            let lease = self
                .backend
                .acquire_restore(&request.instance_id)
                .map_err(|failure| ManagedFailure::Backend(failure.kind()))?;
            return self.launch_managed(&request, Some(LockedLaunch::Workspace(lease.as_ref())));
        }
        self.launch_managed(&request, None)
    }

    /// Returns the recorded name used to check restore ownership after a runner restart.
    /// # Errors
    /// Returns the durable-store failure if ownership cannot be read.
    pub fn restore_owner(
        &mut self,
        instance: &crate::InstanceId,
    ) -> Result<Option<crate::MachineName>, ManagedFailure> {
        use crate::{OperationKind, engine::machine_state::TerminalBasis};

        let Some(stored) = self.load_machine(instance)? else {
            return Ok(None);
        };
        let name = match stored.machine.phase {
            DurablePhase::Terminal {
                operation: OperationKind::Stop,
                basis,
                ..
            } => match *basis {
                TerminalBasis::Active { active } => active.machine_name().cloned(),
                TerminalBasis::Launch { .. } => None,
            },
            DurablePhase::Launching {
                intent,
                prior_stop: Some(_),
            } => intent.machine_name.clone(),
            DurablePhase::Active { active } => active.machine_name().cloned(),
            DurablePhase::Recovery {
                source,
                installed: true,
            } => Some(source.owner.clone()),
            _ => None,
        };
        Ok(name)
    }

    /// Restores a checkpoint created for the same stopped Instance.
    /// # Errors
    /// Returns a typed failure if the stopped record, admitted Generation, or checkpoint cannot
    /// be confirmed, or the backend cannot prove readiness.
    #[allow(clippy::needless_pass_by_value)]
    pub fn restore_machine(
        &mut self,
        request: LaunchMachineRequest,
        checkpoint: &crate::OperationId,
    ) -> Result<MachineLaunch, ManagedFailure> {
        self.launch_from_checkpoint(&request, checkpoint, None)
    }

    fn launch_from_checkpoint(
        &mut self,
        request: &LaunchMachineRequest,
        checkpoint: &crate::OperationId,
        fork_source: Option<&crate::InstanceId>,
    ) -> Result<MachineLaunch, ManagedFailure> {
        if request.operation_id == *checkpoint
            || request.volume.is_some()
            || request.windowed
            || (fork_source.is_some()
                && *request.shape.capabilities().network_policy()
                    != crate::NetworkPolicy::isolated())
        {
            return Err(ManagedFailure::State(ManagedStateError::OperationConflict));
        }
        let lease = self
            .backend
            .acquire_restore(&request.instance_id)
            .map_err(|failure| ManagedFailure::Backend(failure.kind()))?;
        self.launch_managed(
            request,
            Some(LockedLaunch::Checkpoint {
                checkpoint,
                fork_source,
                lease: lease.as_ref(),
            }),
        )
    }
}
