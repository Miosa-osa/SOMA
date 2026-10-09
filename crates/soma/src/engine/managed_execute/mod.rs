mod admission;
mod disposition;
mod failure;
mod receipt;

use crate::{
    Backend, BackendFailureKind, CleanupEvidence, CommandStatus, Milestone, MilestoneKind,
    Observation, StateStore,
};

use super::{
    Engine, ExecuteMachineRequest, MachineExecution, ManagedFailure, RunFailureKind,
    machine_state::{DurableMachine, DurablePhase, ExecutionTombstone},
    run_evidence::{append_command, append_failure, terminal_status, validate_command},
};

use self::{
    disposition::CommandDisposition,
    receipt::{execution_receipt, store_operation_failure},
};

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
                return Err(self.fail_execution(
                    &request,
                    admission,
                    milestones,
                    kind,
                    None,
                    refusal_disposition(failure.kind()),
                ));
            }
        };
        let Some(validated) = validated else {
            return Err(self.fail_execution(
                &request,
                admission,
                milestones,
                RunFailureKind::ObservationMismatch,
                None,
                CommandDisposition::Unobserved,
            ));
        };
        append_command(&mut milestones, validated.times);
        let status = validated.status;
        // A status the guest agent sent is proof the agent lived to send it, and the agent kills
        // and reaps a command's whole process group before it reports one. So an outcome that
        // did not succeed decides what the caller reads, and never costs the machine.
        if let Some((kind, disposition)) = stopped_command(status) {
            return Err(self.fail_execution(
                &request,
                admission,
                milestones,
                kind,
                Some((validated.output, validated.metadata, status)),
                disposition,
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

/// What one backend failure leaves behind.
///
/// A workload the runtime refuses is a request it would not carry to the machine at all, so the
/// machine was never addressed. Every other backend failure is either one the machine was in
/// the middle of or one whose state nobody observed, and an unknown machine is released.
const fn refusal_disposition(kind: BackendFailureKind) -> CommandDisposition {
    match kind {
        BackendFailureKind::WorkloadRejected => CommandDisposition::RefusedBeforeRun,
        _ => CommandDisposition::Unobserved,
    }
}

/// What one guest-reported status means, when the command did not simply finish.
///
/// `None` is a command whose result the caller reads: an exit, or a signal that ended it. Those
/// two are what a completed execution may record, so they continue along the success path and
/// their sandbox is kept for the same reason every other keeps it, namely that the guest agent
/// reported them.
const fn stopped_command(status: CommandStatus) -> Option<(RunFailureKind, CommandDisposition)> {
    match status {
        CommandStatus::Exited { .. } | CommandStatus::Signaled { .. } => None,
        CommandStatus::TimedOut => Some((
            RunFailureKind::TimedOut,
            CommandDisposition::StoppedByTheGuest,
        )),
        CommandStatus::OutputLimitExceeded => Some((
            RunFailureKind::OutputLimitExceeded,
            CommandDisposition::StoppedByTheGuest,
        )),
        CommandStatus::SpawnFailed { errno } => Some((
            RunFailureKind::SpawnFailed { errno },
            CommandDisposition::SpawnRefused,
        )),
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
