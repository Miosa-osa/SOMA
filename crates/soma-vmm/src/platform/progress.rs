#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RestoreStep {
    ArtifactsVerified,
    MachineRestored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessStep {
    GuestAuthenticated,
    GenerationAcknowledged,
    IdentityRepaired,
    NetworkRepaired,
}

pub(super) const RESTORE_SEQUENCE: [RestoreStep; 2] =
    [RestoreStep::ArtifactsVerified, RestoreStep::MachineRestored];
pub(super) const READINESS_SEQUENCE: [ReadinessStep; 4] = [
    ReadinessStep::GuestAuthenticated,
    ReadinessStep::GenerationAcknowledged,
    ReadinessStep::IdentityRepaired,
    ReadinessStep::NetworkRepaired,
];

#[derive(Clone, Copy)]
struct ProgressTrace<Step: Copy, const CAPACITY: usize> {
    steps: [Option<Step>; CAPACITY],
    observed: usize,
}

impl<Step: Copy + Eq, const CAPACITY: usize> ProgressTrace<Step, CAPACITY> {
    fn from_steps<const COUNT: usize>(steps: [Step; COUNT]) -> Self {
        let mut trace = Self {
            steps: [None; CAPACITY],
            observed: COUNT,
        };
        for (index, step) in steps.into_iter().take(CAPACITY).enumerate() {
            trace.steps[index] = Some(step);
        }
        trace
    }

    fn assess(self, expected: [Step; CAPACITY]) -> ProgressAssessment {
        let comparable = self.observed.min(CAPACITY);
        let mut matched = 0;
        while matched < comparable && self.steps[matched] == Some(expected[matched]) {
            matched += 1;
        }
        ProgressAssessment {
            matched,
            observed: self.observed,
            capacity: CAPACITY,
            ordered: self.observed <= CAPACITY && matched == self.observed,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ProgressAssessment {
    pub(super) matched: usize,
    pub(super) observed: usize,
    capacity: usize,
    ordered: bool,
}

impl ProgressAssessment {
    pub(super) const fn is_complete(self) -> bool {
        self.ordered && self.observed == self.capacity
    }

    pub(super) const fn is_ordered_prefix(self) -> bool {
        self.ordered
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RestoreProgress(ProgressTrace<RestoreStep, 2>);

impl RestoreProgress {
    pub(crate) fn from_steps<const COUNT: usize>(steps: [RestoreStep; COUNT]) -> Self {
        Self(ProgressTrace::from_steps(steps))
    }

    pub(super) fn assess(self) -> ProgressAssessment {
        self.0.assess(RESTORE_SEQUENCE)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ReadinessProgress(ProgressTrace<ReadinessStep, 4>);

impl ReadinessProgress {
    pub(crate) fn from_steps<const COUNT: usize>(steps: [ReadinessStep; COUNT]) -> Self {
        Self(ProgressTrace::from_steps(steps))
    }

    pub(super) fn assess(self) -> ProgressAssessment {
        self.0.assess(READINESS_SEQUENCE)
    }
}
