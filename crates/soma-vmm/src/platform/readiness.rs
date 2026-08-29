use crate::{ExitStatus, Recovery};

use super::progress::{ProgressAssessment, READINESS_SEQUENCE, ReadinessProgress};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessFailurePoint {
    GuestAuthentication,
    GenerationAcknowledgement,
    IdentityRepair,
    NetworkRepair,
    ReadinessProbe,
}

/// Type-state authority for a repaired guest with a successful readiness command.
pub(crate) struct ReadyAuthenticatedGuest {
    _private: (),
}

impl ReadyAuthenticatedGuest {
    pub(super) fn from_observation(
        progress: ReadinessProgress,
        readiness_status: ExitStatus,
    ) -> Result<Self, ReadinessFailure> {
        let assessment = progress.assess();
        if !assessment.is_complete() {
            return Err(ReadinessFailure::from_progress(
                progress,
                Recovery::RepairHost,
            ));
        }
        if readiness_status != ExitStatus::Code(0) {
            return Err(ReadinessFailure::at(
                ReadinessFailurePoint::ReadinessProbe,
                Recovery::ReplaceMachine,
            ));
        }
        Ok(Self { _private: () })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        progress: ReadinessProgress,
        readiness_status: ExitStatus,
    ) -> Result<Self, ReadinessFailure> {
        Self::from_observation(progress, readiness_status)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ReadinessFailure {
    point: ReadinessFailurePoint,
    recovery: Recovery,
}

impl ReadinessFailure {
    const fn at(point: ReadinessFailurePoint, recovery: Recovery) -> Self {
        Self { point, recovery }
    }

    fn from_progress(progress: ReadinessProgress, recovery: Recovery) -> Self {
        let assessment = progress.assess();
        if assessment.is_ordered_prefix() && assessment.observed <= READINESS_SEQUENCE.len() {
            Self::at(readiness_point(assessment.observed), recovery)
        } else {
            Self::contract_violation(assessment)
        }
    }

    fn contract_violation(assessment: ProgressAssessment) -> Self {
        Self::at(readiness_point(assessment.matched), Recovery::RepairHost)
    }

    #[cfg(test)]
    pub(crate) fn for_test(progress: ReadinessProgress, recovery: Recovery) -> Self {
        Self::from_progress(progress, recovery)
    }

    pub(crate) const fn point(self) -> ReadinessFailurePoint {
        self.point
    }

    pub(crate) const fn completed(self) -> usize {
        match self.point {
            ReadinessFailurePoint::GuestAuthentication => 0,
            ReadinessFailurePoint::GenerationAcknowledgement => 1,
            ReadinessFailurePoint::IdentityRepair => 2,
            ReadinessFailurePoint::NetworkRepair => 3,
            ReadinessFailurePoint::ReadinessProbe => 4,
        }
    }

    pub(crate) const fn recovery(self) -> Recovery {
        self.recovery
    }
}

const fn readiness_point(index: usize) -> ReadinessFailurePoint {
    match index {
        0 => ReadinessFailurePoint::GuestAuthentication,
        1 => ReadinessFailurePoint::GenerationAcknowledgement,
        2 => ReadinessFailurePoint::IdentityRepair,
        3 => ReadinessFailurePoint::NetworkRepair,
        _ => ReadinessFailurePoint::ReadinessProbe,
    }
}
