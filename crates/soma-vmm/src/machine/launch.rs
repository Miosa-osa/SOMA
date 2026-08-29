use super::{
    Machine, State,
    fault::{lifecycle_failure, operation_capacity_exceeded, operation_conflict},
};
use crate::{
    CleanupEvidence, Failure, FailureKind, FailurePhase, Launch, Milestone, Milestones, Ready,
    Recovery,
    platform::{
        ReadinessFailure, ReadinessFailurePoint, ReadyAuthenticatedGuest, RestoreFailure,
        RestoreFailurePoint, RestoredMachine,
    },
};

const RESTORE_MILESTONES: [Milestone; 2] =
    [Milestone::ArtifactsVerified, Milestone::MachineRestored];
const READINESS_MILESTONES: [Milestone; 4] = [
    Milestone::GuestAuthenticated,
    Milestone::GenerationAcknowledged,
    Milestone::IdentityRepaired,
    Milestone::NetworkRepaired,
];

impl Machine {
    /// Launches one Instance and returns only after authenticated command readiness.
    ///
    /// # Errors
    ///
    /// Returns a typed [`Failure`] when lifecycle, verification, restore, authentication, Repair,
    /// readiness, or rollback cannot satisfy the contract.
    pub fn launch(&mut self, request: Launch) -> Result<Ready, Failure> {
        match self.operations.replay_launch(&request) {
            Ok(Some(outcome)) => return outcome,
            Err(_) => return Err(operation_conflict()),
            Ok(None) => {}
        }
        if self.state != State::New {
            return Err(lifecycle_failure());
        }
        self.operations
            .ensure_launch_capacity()
            .map_err(|_| operation_capacity_exceeded())?;

        let outcome = self.perform_launch(&request);
        self.operations.record_launch(request, outcome.clone());
        outcome
    }

    fn perform_launch(&mut self, request: &Launch) -> Result<Ready, Failure> {
        self.state = State::Launching;
        self.instance_id = Some(request.instance_id());
        let mut milestones = Milestones::default();
        milestones.push(Milestone::RequestAccepted);

        let restored = self.verify_and_restore(request, &mut milestones)?;
        let guest = self.authenticate_repair_and_ready(request, restored, &mut milestones)?;

        milestones.push(Milestone::FirstCommandSucceeded);
        milestones.push(Milestone::Ready);
        self.state = State::Ready;
        self.guest = Some(guest);
        Ok(Ready::new(
            request.operation_id(),
            request.instance_id(),
            request.generation().id(),
            request.generation().machine(),
            milestones,
        ))
    }

    fn verify_and_restore(
        &mut self,
        request: &Launch,
        milestones: &mut Milestones,
    ) -> Result<RestoredMachine, Failure> {
        match self.platform.verify_and_restore(request) {
            Ok(restored) => {
                append_milestones(milestones, &RESTORE_MILESTONES, RESTORE_MILESTONES.len());
                Ok(restored)
            }
            Err(failure) => {
                append_milestones(milestones, &RESTORE_MILESTONES, failure.completed());
                Err(self.rollback_launch_failure(
                    request,
                    restore_failure(failure),
                    failure.recovery(),
                    milestones,
                ))
            }
        }
    }

    fn authenticate_repair_and_ready(
        &mut self,
        request: &Launch,
        restored: RestoredMachine,
        milestones: &mut Milestones,
    ) -> Result<ReadyAuthenticatedGuest, Failure> {
        match self
            .platform
            .authenticate_repair_and_ready(request, restored)
        {
            Ok(guest) => {
                append_milestones(
                    milestones,
                    &READINESS_MILESTONES,
                    READINESS_MILESTONES.len(),
                );
                Ok(guest)
            }
            Err(failure) => {
                append_milestones(milestones, &READINESS_MILESTONES, failure.completed());
                Err(self.rollback_launch_failure(
                    request,
                    readiness_failure(failure),
                    failure.recovery(),
                    milestones,
                ))
            }
        }
    }

    fn rollback_launch_failure(
        &mut self,
        request: &Launch,
        (kind, phase): (FailureKind, FailurePhase),
        recovery: Recovery,
        milestones: &mut Milestones,
    ) -> Failure {
        milestones.push(Milestone::RollbackStarted);
        let (recovery, cleanup) = match self.platform.rollback(request) {
            Ok(()) => {
                milestones.push(Milestone::CleanupCompleted);
                (recovery, CleanupEvidence::Complete)
            }
            Err(rollback) => (rollback.recovery(), CleanupEvidence::Incomplete),
        };
        self.state = State::Failed;
        Failure::new(kind, phase, recovery, milestones.clone(), cleanup)
    }
}

fn append_milestones(target: &mut Milestones, source: &[Milestone], count: usize) {
    for milestone in source.iter().take(count) {
        target.push(*milestone);
    }
}

const fn restore_failure(failure: RestoreFailure) -> (FailureKind, FailurePhase) {
    match failure.point() {
        RestoreFailurePoint::ArtifactVerification => (
            FailureKind::GenerationVerificationFailed,
            FailurePhase::ArtifactVerification,
        ),
        RestoreFailurePoint::Restore => (FailureKind::RestoreFailed, FailurePhase::Restore),
    }
}

const fn readiness_failure(failure: ReadinessFailure) -> (FailureKind, FailurePhase) {
    match failure.point() {
        ReadinessFailurePoint::GuestAuthentication => (
            FailureKind::GuestAuthenticationFailed,
            FailurePhase::GuestAuthentication,
        ),
        ReadinessFailurePoint::GenerationAcknowledgement => (
            FailureKind::GenerationAcknowledgementFailed,
            FailurePhase::GenerationAcknowledgement,
        ),
        ReadinessFailurePoint::IdentityRepair => (
            FailureKind::IdentityRepairFailed,
            FailurePhase::IdentityRepair,
        ),
        ReadinessFailurePoint::NetworkRepair => (
            FailureKind::NetworkRepairFailed,
            FailurePhase::NetworkRepair,
        ),
        ReadinessFailurePoint::ReadinessProbe => (
            FailureKind::ReadinessProbeFailed,
            FailurePhase::ReadinessProbe,
        ),
    }
}
