use crate::{
    Backend, CommandStatus, Milestone, MilestoneKind, RunRequest, WorkloadEvidence, fingerprint,
};

use super::{
    Engine, FailurePhase, RunFailure, RunFailureKind, RunOutcome,
    run_evidence::{
        FailureContext, ReceiptContext, append_command, append_failure, append_launch,
        append_milestone, successful_receipt, terminal_status, validate_command, validate_launch,
    },
};

impl<B: Backend, S> Engine<B, S> {
    /// Resolves, launches, executes, and releases one bounded one-shot Machine transaction.
    ///
    /// # Errors
    ///
    /// Returns [`RunFailure`] with a partial validated receipt after any failed phase.
    #[allow(
        clippy::needless_pass_by_value,
        clippy::too_many_lines,
        reason = "the owned request drives one explicit end-to-end transaction"
    )]
    pub fn run(&mut self, request: RunRequest) -> Result<RunOutcome, RunFailure> {
        let (operation_id, instance_id, image, shape, command, limits, machine_name) =
            request.parts();
        let operation_id = operation_id.clone();
        let instance_id = instance_id.clone();
        let machine_name = machine_name.cloned();
        let source_fingerprint = fingerprint::source(image);
        let mut milestones = vec![Milestone::new(MilestoneKind::Accepted, 0)];

        let resolution = self.backend.resolve(crate::ResolutionRequest::new(
            &operation_id,
            image,
            &source_fingerprint,
        ));
        let (workload, prepared, resolved_at_ns) = match resolution {
            Ok(observation) => {
                let (observed_operation, observed_source, workload, prepared, elapsed) =
                    observation.into_parts();
                if observed_operation != operation_id
                    || observed_source != source_fingerprint
                    || !append_milestone(&mut milestones, MilestoneKind::WorkloadResolved, elapsed)
                {
                    return Err(self.failure_without_cleanup(FailureContext {
                        kind: RunFailureKind::ObservationMismatch,
                        operation_id,
                        instance_id,
                        machine_name: machine_name.clone(),
                        fingerprint: source_fingerprint.clone(),
                        workload: WorkloadEvidence::Unresolved { source_fingerprint },
                        requested_shape: shape.clone(),
                        milestones,
                    }));
                }
                (workload, prepared, elapsed)
            }
            Err(failure) => {
                let failure_kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: FailurePhase::Resolution,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                return Err(self.failure_without_cleanup(FailureContext {
                    kind: failure_kind,
                    operation_id,
                    instance_id,
                    machine_name: machine_name.clone(),
                    fingerprint: source_fingerprint.clone(),
                    workload: WorkloadEvidence::Unresolved { source_fingerprint },
                    requested_shape: shape.clone(),
                    milestones,
                }));
            }
        };

        let request_fingerprint = fingerprint::run(
            &workload,
            &instance_id,
            machine_name.as_ref(),
            shape,
            command,
            limits,
        );
        let launch = self.backend.launch(crate::LaunchRequest::new(
            &operation_id,
            &instance_id,
            &workload,
            &prepared,
            shape,
        ));
        let launch_evidence = match launch {
            Ok(observation) => match validate_launch(
                observation,
                &operation_id,
                &instance_id,
                &workload,
                self.backend.kind(),
                shape,
                resolved_at_ns,
            ) {
                Some(evidence) => {
                    append_launch(&mut milestones, evidence.times);
                    evidence
                }
                None => {
                    return Err(self.failure_with_cleanup(
                        FailureContext {
                            kind: RunFailureKind::ObservationMismatch,
                            operation_id,
                            instance_id,
                            machine_name: machine_name.clone(),
                            fingerprint: request_fingerprint,
                            workload: WorkloadEvidence::Resolved { identity: workload },
                            requested_shape: shape.clone(),
                            milestones,
                        },
                        crate::CleanupReason::Rollback,
                        None,
                    ));
                }
            },
            Err(failure) => {
                let failure_kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: FailurePhase::Launch,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                return Err(self.failure_with_cleanup(
                    FailureContext {
                        kind: failure_kind,
                        operation_id,
                        instance_id,
                        machine_name: machine_name.clone(),
                        fingerprint: request_fingerprint,
                        workload: WorkloadEvidence::Resolved { identity: workload },
                        requested_shape: shape.clone(),
                        milestones,
                    },
                    crate::CleanupReason::Rollback,
                    None,
                ));
            }
        };

        let command_result = self.backend.execute(crate::ExecutionRequest::new(
            &operation_id,
            &instance_id,
            command,
            limits,
        ));
        let (status, output, metadata) = match command_result {
            Ok(observation) => match validate_command(
                observation,
                &operation_id,
                &instance_id,
                limits,
                launch_evidence.times.values()[2],
            ) {
                Some(validated) => {
                    append_command(&mut milestones, validated.times);
                    (validated.status, validated.output, validated.metadata)
                }
                None => {
                    return Err(self.failure_with_cleanup(
                        FailureContext {
                            kind: RunFailureKind::ObservationMismatch,
                            operation_id,
                            instance_id,
                            machine_name: machine_name.clone(),
                            fingerprint: request_fingerprint,
                            workload: WorkloadEvidence::Resolved { identity: workload },
                            requested_shape: shape.clone(),
                            milestones,
                        },
                        crate::CleanupReason::Rollback,
                        Some(launch_evidence),
                    ));
                }
            },
            Err(failure) => {
                let failure_kind = if append_failure(&mut milestones, failure) {
                    RunFailureKind::Backend {
                        phase: FailurePhase::Command,
                        kind: failure.kind(),
                    }
                } else {
                    RunFailureKind::ObservationMismatch
                };
                return Err(self.failure_with_cleanup(
                    FailureContext {
                        kind: failure_kind,
                        operation_id,
                        instance_id,
                        machine_name: machine_name.clone(),
                        fingerprint: request_fingerprint,
                        workload: WorkloadEvidence::Resolved { identity: workload },
                        requested_shape: shape.clone(),
                        milestones,
                    },
                    crate::CleanupReason::Rollback,
                    Some(launch_evidence),
                ));
            }
        };

        let cleanup = self.perform_cleanup(
            &operation_id,
            &instance_id,
            crate::CleanupReason::RunCompleted,
            &mut milestones,
        );
        let terminal_status = terminal_status(status);
        let receipt = successful_receipt(ReceiptContext {
            operation_id,
            instance_id,
            machine_name,
            fingerprint: request_fingerprint,
            workload,
            requested_shape: shape.clone(),
            launch: launch_evidence,
            milestones,
            terminal_status,
            output: metadata,
            cleanup: cleanup.evidence,
        });
        let outcome = RunOutcome { receipt, output };

        if !cleanup.complete {
            return Err(RunFailure {
                kind: cleanup
                    .failure_kind
                    .unwrap_or(RunFailureKind::CleanupIncomplete),
                receipt: Box::new(outcome.receipt),
                output: Some(outcome.output),
            });
        }
        match status {
            CommandStatus::Exited { .. } | CommandStatus::Signaled { .. } => Ok(outcome),
            CommandStatus::TimedOut => Err(RunFailure {
                kind: RunFailureKind::TimedOut,
                receipt: Box::new(outcome.receipt),
                output: Some(outcome.output),
            }),
            CommandStatus::OutputLimitExceeded => Err(RunFailure {
                kind: RunFailureKind::OutputLimitExceeded,
                receipt: Box::new(outcome.receipt),
                output: Some(outcome.output),
            }),
        }
    }
}
