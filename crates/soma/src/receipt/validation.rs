use crate::{
    CleanupMethod, MeasurementClass, Milestone, MilestoneKind, Observation, OperationKind,
    TerminalStatus, WorkloadEvidence,
};

use super::{ExecutionReceipt, ReceiptValidationError, SCHEMA_VERSION};

impl ExecutionReceipt {
    /// Validates the complete receipt schema and all cross-field invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptValidationError`] when identity, sequence, terminal, measurement,
    /// workload, shape, output, or cleanup evidence is internally inconsistent.
    pub fn validate(&self) -> Result<(), ReceiptValidationError> {
        if self.schema_version != SCHEMA_VERSION
            || self.soma_version.is_empty()
            || self.milestones.first() != Some(&Milestone::new(MilestoneKind::Accepted, 0))
            || !self
                .milestones
                .windows(2)
                .all(|pair| pair[0].elapsed_ns() <= pair[1].elapsed_ns())
            || !self.allowed_milestones()
            || !self.phase_dependencies_hold()
            || !self.terminal_is_consistent()
            || !self.workload_is_consistent()
            || !self.effective_shape.matches_request(&self.requested_shape)
            || !self
                .effective_network
                .matches_request(self.requested_shape.capabilities().network_policy())
            || !self.measurement_is_consistent()
        {
            return Err(ReceiptValidationError);
        }
        if let Observation::Observed(output) = &self.output
            && (!matches!(self.operation, OperationKind::Run | OperationKind::Execute)
                || !output.is_valid())
        {
            return Err(ReceiptValidationError);
        }
        Ok(())
    }

    fn allowed_milestones(&self) -> bool {
        if self.milestones.len() > 10 {
            return false;
        }
        let mut seen = [false; 11];
        let mut last_rank = 0;
        let mut failed = false;
        for (index, milestone) in self.milestones.iter().enumerate() {
            let kind = milestone.kind();
            let slot = milestone_slot(kind);
            if seen[slot]
                || (index == 0) != (kind == MilestoneKind::Accepted)
                || !milestone_allowed(self.operation, kind)
            {
                return false;
            }
            seen[slot] = true;
            if kind == MilestoneKind::FailureObserved {
                failed = true;
                continue;
            }
            if failed
                && !matches!(
                    kind,
                    MilestoneKind::CleanupStarted | MilestoneKind::CleanupFinished
                )
            {
                return false;
            }
            let rank = milestone_rank(kind);
            if rank < last_rank {
                return false;
            }
            last_rank = rank;
        }
        true
    }

    fn phase_dependencies_hold(&self) -> bool {
        (!self.has(MilestoneKind::Admitted) || self.has(MilestoneKind::WorkloadResolved))
            && (!self.has(MilestoneKind::MachineLaunched) || self.has(MilestoneKind::Admitted))
            && (!self.has(MilestoneKind::Ready) || self.has(MilestoneKind::MachineLaunched))
            && (!self.has(MilestoneKind::CommandStarted)
                || self.operation != OperationKind::Run
                || self.has(MilestoneKind::Ready))
            && (!self.has(MilestoneKind::CommandFinished)
                || self.has(MilestoneKind::CommandStarted))
            && (!self.has(MilestoneKind::CleanupStarted)
                || !matches!(self.operation, OperationKind::Run | OperationKind::Launch)
                || self.has(MilestoneKind::WorkloadResolved))
            && (!self.has(MilestoneKind::CleanupFinished)
                || self.has(MilestoneKind::CleanupStarted))
    }

    fn terminal_is_consistent(&self) -> bool {
        match self.terminal_status {
            TerminalStatus::Ready => {
                self.operation == OperationKind::Launch
                    && self.has_launch_chain()
                    && self.cleanup.all_not_owned()
            }
            TerminalStatus::Stopped => {
                self.operation == OperationKind::Stop
                    && self.has(MilestoneKind::CleanupFinished)
                    && self.cleanup.is_complete()
                    && matches!(
                        self.cleanup.method(),
                        CleanupMethod::Graceful
                            | CleanupMethod::GracefulThenForced
                            | CleanupMethod::NotApplicable
                    )
            }
            TerminalStatus::Destroyed => {
                self.operation == OperationKind::Destroy
                    && self.has(MilestoneKind::CleanupFinished)
                    && self.cleanup.is_complete()
                    && matches!(
                        self.cleanup.method(),
                        CleanupMethod::Forced | CleanupMethod::NotApplicable
                    )
            }
            TerminalStatus::Inspected { .. } => {
                self.operation == OperationKind::Inspect
                    && self.has(MilestoneKind::Inspected)
                    && self.cleanup.all_not_owned()
            }
            TerminalStatus::Exited { .. } => match self.operation {
                OperationKind::Run => self.has_launch_chain() && self.has_command_chain(),
                OperationKind::Execute => self.has_command_chain(),
                _ => false,
            },
            TerminalStatus::Signaled { .. }
            | TerminalStatus::TimedOut
            | TerminalStatus::OutputLimitExceeded => {
                (match self.operation {
                    OperationKind::Run => self.has_launch_chain() && self.has_command_chain(),
                    OperationKind::Execute => self.has_command_chain(),
                    _ => false,
                }) && !self.cleanup.all_not_owned()
            }
            TerminalStatus::Failed => true,
        }
    }

    fn workload_is_consistent(&self) -> bool {
        let identity_is_consistent = match (&self.workload, &self.digest_binding) {
            (WorkloadEvidence::Unresolved { .. }, Observation::Observed(_)) => false,
            (WorkloadEvidence::Unresolved { .. }, _) => {
                self.effective_shape.all_unavailable() && self.effective_network.all_unavailable()
            }
            (WorkloadEvidence::Resolved { .. }, Observation::Observed(_)) => {
                matches!(self.isolation, Observation::Observed(_))
                    && matches!(self.preparation, Observation::Observed(_))
            }
            (WorkloadEvidence::Resolved { .. }, Observation::Unavailable(_)) => true,
        };
        identity_is_consistent
            && (!matches!(self.operation, OperationKind::Run | OperationKind::Launch)
                || self.has(MilestoneKind::WorkloadResolved)
                    == matches!(self.workload, WorkloadEvidence::Resolved { .. }))
    }

    fn measurement_is_consistent(&self) -> bool {
        matches!(
            (self.operation, self.measurement.class()),
            (OperationKind::Run, MeasurementClass::FacadeRunEndToEnd)
                | (OperationKind::Launch, MeasurementClass::FacadeManagedLaunch)
                | (
                    OperationKind::Execute,
                    MeasurementClass::FacadeManagedCommand
                )
                | (OperationKind::Stop, MeasurementClass::FacadeManagedStop)
                | (
                    OperationKind::Inspect,
                    MeasurementClass::FacadeManagedInspect
                )
                | (
                    OperationKind::Destroy,
                    MeasurementClass::FacadeManagedDestroy
                )
        )
    }

    fn has_launch_chain(&self) -> bool {
        self.has(MilestoneKind::WorkloadResolved)
            && self.has(MilestoneKind::Admitted)
            && self.has(MilestoneKind::MachineLaunched)
            && self.has(MilestoneKind::Ready)
    }

    fn has_command_chain(&self) -> bool {
        self.has(MilestoneKind::CommandStarted) && self.has(MilestoneKind::CommandFinished)
    }

    fn has(&self, kind: MilestoneKind) -> bool {
        self.milestones.iter().any(|value| value.kind() == kind)
    }
}

fn milestone_allowed(operation: OperationKind, kind: MilestoneKind) -> bool {
    match operation {
        OperationKind::Run => kind != MilestoneKind::Inspected,
        OperationKind::Launch => matches!(
            kind,
            MilestoneKind::Accepted
                | MilestoneKind::WorkloadResolved
                | MilestoneKind::Admitted
                | MilestoneKind::MachineLaunched
                | MilestoneKind::Ready
                | MilestoneKind::FailureObserved
                | MilestoneKind::CleanupStarted
                | MilestoneKind::CleanupFinished
        ),
        OperationKind::Execute => matches!(
            kind,
            MilestoneKind::Accepted
                | MilestoneKind::CommandStarted
                | MilestoneKind::CommandFinished
                | MilestoneKind::FailureObserved
                | MilestoneKind::CleanupStarted
                | MilestoneKind::CleanupFinished
        ),
        OperationKind::Stop | OperationKind::Destroy => matches!(
            kind,
            MilestoneKind::Accepted
                | MilestoneKind::CleanupStarted
                | MilestoneKind::CleanupFinished
                | MilestoneKind::FailureObserved
        ),
        OperationKind::Inspect => matches!(
            kind,
            MilestoneKind::Accepted | MilestoneKind::Inspected | MilestoneKind::FailureObserved
        ),
    }
}

const fn milestone_rank(kind: MilestoneKind) -> u8 {
    match kind {
        MilestoneKind::Accepted => 0,
        MilestoneKind::WorkloadResolved | MilestoneKind::Inspected => 1,
        MilestoneKind::Admitted => 2,
        MilestoneKind::MachineLaunched => 3,
        MilestoneKind::Ready => 4,
        MilestoneKind::CommandStarted => 5,
        MilestoneKind::CommandFinished => 6,
        MilestoneKind::CleanupStarted => 7,
        MilestoneKind::CleanupFinished => 8,
        MilestoneKind::FailureObserved => 9,
    }
}

const fn milestone_slot(kind: MilestoneKind) -> usize {
    match kind {
        MilestoneKind::Accepted => 0,
        MilestoneKind::WorkloadResolved => 1,
        MilestoneKind::Admitted => 2,
        MilestoneKind::MachineLaunched => 3,
        MilestoneKind::Ready => 4,
        MilestoneKind::CommandStarted => 5,
        MilestoneKind::CommandFinished => 6,
        MilestoneKind::CleanupStarted => 7,
        MilestoneKind::CleanupFinished => 8,
        MilestoneKind::FailureObserved => 9,
        MilestoneKind::Inspected => 10,
    }
}
