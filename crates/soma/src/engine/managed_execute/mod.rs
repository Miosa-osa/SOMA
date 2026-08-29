mod admission;
mod failure;
mod receipt;

use crate::{
    Backend, CleanupEvidence, CommandStatus, Milestone, MilestoneKind, Observation, StateStore,
};

use super::{
    Engine, ExecuteMachineRequest, MachineExecution, ManagedFailure, RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, ExecutionTombstone},
    run_evidence::{append_command, append_failure, terminal_status, validate_command},
};

use self::receipt::{execution_receipt, store_operation_failure};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Executes one bounded direct command against an exact managed Instance.
    ///
    /// # Errors
    ///
    /// Returns typed state, durable-store, replay, or evidence-carrying operation failures.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the use-case boundary takes ownership of its immutable request"
    )]
    pub fn execute_machine(
        &mut self,
        request: ExecuteMachineRequest,
    ) -> Result<MachineExecution, ManagedFailure> {
        let admission = self.admit_execution(&request)?;
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];
        let observation = self.backend.execute(crate::ExecutionRequest::new(
            &request.operation_id,
            &request.instance_id,
            &request.command,
            &request.limits,
        ));
        let validated = match observation {
            Ok(observation) => validate_command(
                observation,
                &request.operation_id,
                &request.instance_id,
                &request.limits,
                0,
            ),
            Err(failure) => {
                let kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: super::FailurePhase::Command,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                return Err(self.fail_execution(&request, admission, milestones, kind, None));
            }
        };
        let Some(validated) = validated else {
            return Err(self.fail_execution(
                &request,
                admission,
                milestones,
                RunFailureKind::ObservationMismatch,
                None,
            ));
        };
        append_command(&mut milestones, validated.times);
        let status = validated.status;
        if matches!(
            status,
            CommandStatus::Signaled { .. }
                | CommandStatus::TimedOut
                | CommandStatus::OutputLimitExceeded
        ) {
            let kind = match status {
                CommandStatus::Signaled { .. } => RunFailureKind::Interrupted,
                CommandStatus::TimedOut => RunFailureKind::TimedOut,
                CommandStatus::OutputLimitExceeded => RunFailureKind::OutputLimitExceeded,
                CommandStatus::Exited { .. } => unreachable!(),
            };
            return Err(self.fail_execution(
                &request,
                admission,
                milestones,
                kind,
                Some((validated.output, validated.metadata, status)),
            ));
        }
        let receipt = execution_receipt(
            &request.operation_id,
            &request.instance_id,
            &admission.active,
            admission.fingerprint.clone(),
            milestones,
            terminal_status(status),
            Observation::Observed(validated.metadata),
            CleanupEvidence::not_owned(),
        );
        let tombstone = ExecutionTombstone::from_receipt(&receipt)
            .map_err(|failure| ManagedFailure::StateStore(failure.kind()))?;
        let mut active = admission.active;
        active.completed_executions.push(tombstone);
        let next = durable_with_phase(
            &request.instance_id,
            DurablePhase::Active {
                active: Box::new(active),
            },
        );
        if let Err(failure) = self.replace_machine(admission.revision, &next) {
            return Err(store_operation_failure(
                &failure,
                receipt,
                Some(validated.output),
            ));
        }
        Ok(MachineExecution {
            receipt,
            output: validated.output,
        })
    }
}

fn durable_with_phase(instance_id: &crate::InstanceId, phase: DurablePhase) -> DurableMachine {
    let launch = match &phase {
        DurablePhase::Active { active }
        | DurablePhase::Executing { active, .. }
        | DurablePhase::Terminating { active, .. } => active.launch_receipt.clone(),
        DurablePhase::Terminal { basis, .. } => match basis.as_ref() {
            super::machine_state::TerminalBasis::Active { active } => active.launch_receipt.clone(),
            super::machine_state::TerminalBasis::Launch { .. } => {
                unreachable!("managed active transitions retain launch evidence")
            }
        },
        DurablePhase::Launching { .. } => {
            unreachable!("managed active transitions retain launch evidence")
        }
    };
    let mut machine = DurableMachine::active(instance_id.clone(), launch);
    machine.phase = phase;
    machine
}
