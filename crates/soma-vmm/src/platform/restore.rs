use crate::Recovery;

use super::progress::{ProgressAssessment, RESTORE_SEQUENCE, RestoreProgress};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RestoreFailurePoint {
    ArtifactVerification,
    Restore,
}

/// Type-state evidence that the exact Generation was verified and its machine restored.
pub(crate) struct RestoredMachine {
    _private: (),
}

impl RestoredMachine {
    pub(super) fn from_observation(progress: RestoreProgress) -> Result<Self, RestoreFailure> {
        let assessment = progress.assess();
        if assessment.is_complete() {
            Ok(Self { _private: () })
        } else {
            Err(RestoreFailure::from_progress(
                progress,
                Recovery::RepairHost,
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(progress: RestoreProgress) -> Result<Self, RestoreFailure> {
        Self::from_observation(progress)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RestoreFailure {
    point: RestoreFailurePoint,
    recovery: Recovery,
}

impl RestoreFailure {
    const fn at(point: RestoreFailurePoint, recovery: Recovery) -> Self {
        Self { point, recovery }
    }

    fn from_progress(progress: RestoreProgress, recovery: Recovery) -> Self {
        let assessment = progress.assess();
        if assessment.is_ordered_prefix() && assessment.observed < RESTORE_SEQUENCE.len() {
            Self::at(restore_point(assessment.observed), recovery)
        } else {
            Self::contract_violation(assessment)
        }
    }

    fn contract_violation(assessment: ProgressAssessment) -> Self {
        Self::at(restore_point(assessment.matched), Recovery::RepairHost)
    }

    #[cfg(test)]
    pub(crate) fn for_test(progress: RestoreProgress, recovery: Recovery) -> Self {
        Self::from_progress(progress, recovery)
    }

    pub(crate) const fn point(self) -> RestoreFailurePoint {
        self.point
    }

    pub(crate) const fn completed(self) -> usize {
        match self.point {
            RestoreFailurePoint::ArtifactVerification => 0,
            RestoreFailurePoint::Restore => 1,
        }
    }

    pub(crate) const fn recovery(self) -> Recovery {
        self.recovery
    }
}

const fn restore_point(index: usize) -> RestoreFailurePoint {
    if index == 0 {
        RestoreFailurePoint::ArtifactVerification
    } else {
        RestoreFailurePoint::Restore
    }
}
