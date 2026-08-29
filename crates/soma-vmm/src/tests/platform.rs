use crate::{
    Execute, ExitStatus, Launch, Recovery, Stop,
    platform::{
        Platform, PlatformExecution, PlatformFailure, PlatformStop, ReadinessFailure,
        ReadinessProgress, ReadinessStep, ReadyAuthenticatedGuest, RestoreFailure, RestoreProgress,
        RestoreStep, RestoredMachine,
    },
};

pub(crate) struct DeterministicPlatform {
    fail_at: Option<TestStage>,
    execution_count: u64,
    rollback_fails: bool,
    stop_failures_remaining: u8,
}

impl DeterministicPlatform {
    pub(crate) const fn healthy() -> Self {
        Self::configured(None, false, 0)
    }

    pub(crate) const fn failing_at(stage: TestStage) -> Self {
        Self::configured(Some(stage), false, 0)
    }

    pub(crate) const fn failing_with_rollback(stage: TestStage) -> Self {
        Self::configured(Some(stage), true, 0)
    }

    pub(crate) const fn oversized_output() -> Self {
        Self::configured(Some(TestStage::OversizedOutput), false, 0)
    }

    pub(crate) const fn failing_stop_once() -> Self {
        Self::configured(None, false, 1)
    }

    pub(crate) const fn out_of_order_ready_evidence() -> Self {
        Self::configured(Some(TestStage::OutOfOrderReadyEvidence), false, 0)
    }

    pub(crate) const fn out_of_order_restore_evidence() -> Self {
        Self::configured(Some(TestStage::OutOfOrderRestoreEvidence), false, 0)
    }

    pub(crate) const fn nonzero_ready_claim() -> Self {
        Self::configured(Some(TestStage::NonzeroReadyClaim), false, 0)
    }

    const fn configured(
        fail_at: Option<TestStage>,
        rollback_fails: bool,
        stop_failures_remaining: u8,
    ) -> Self {
        Self {
            fail_at,
            execution_count: 0,
            rollback_fails,
            stop_failures_remaining,
        }
    }

    fn complete(&self, stage: TestStage) -> Result<(), PlatformFailure> {
        if self.fail_at == Some(stage) {
            Err(platform_failure())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum TestStage {
    VerifyGeneration,
    Restore,
    IdentityRepair,
    GuestAuthentication,
    GenerationAcknowledgement,
    NetworkRepair,
    NoopProbe,
    NonzeroReadyClaim,
    OutOfOrderReadyEvidence,
    OutOfOrderRestoreEvidence,
    OversizedOutput,
    UserExecute,
    Stop,
}

impl Platform for DeterministicPlatform {
    fn verify_and_restore(&mut self, _launch: &Launch) -> Result<RestoredMachine, RestoreFailure> {
        if self.fail_at == Some(TestStage::VerifyGeneration) {
            return Err(RestoreFailure::for_test(
                RestoreProgress::from_steps([]),
                Recovery::ReplaceMachine,
            ));
        }
        if self.fail_at == Some(TestStage::Restore) {
            return Err(RestoreFailure::for_test(
                RestoreProgress::from_steps([RestoreStep::ArtifactsVerified]),
                Recovery::ReplaceMachine,
            ));
        }
        if self.fail_at == Some(TestStage::OutOfOrderRestoreEvidence) {
            return RestoredMachine::for_test(RestoreProgress::from_steps([
                RestoreStep::MachineRestored,
                RestoreStep::ArtifactsVerified,
            ]));
        }
        RestoredMachine::for_test(RestoreProgress::from_steps([
            RestoreStep::ArtifactsVerified,
            RestoreStep::MachineRestored,
        ]))
    }

    fn authenticate_repair_and_ready(
        &mut self,
        _launch: &Launch,
        _restored: RestoredMachine,
    ) -> Result<ReadyAuthenticatedGuest, ReadinessFailure> {
        if self.fail_at == Some(TestStage::GuestAuthentication) {
            return Err(readiness_failure(ReadinessProgress::from_steps([])));
        }
        if self.fail_at == Some(TestStage::GenerationAcknowledgement) {
            return Err(readiness_failure(ReadinessProgress::from_steps([
                ReadinessStep::GuestAuthenticated,
            ])));
        }
        if self.fail_at == Some(TestStage::IdentityRepair) {
            return Err(readiness_failure(ReadinessProgress::from_steps([
                ReadinessStep::GuestAuthenticated,
                ReadinessStep::GenerationAcknowledged,
            ])));
        }
        if self.fail_at == Some(TestStage::NetworkRepair) {
            return Err(readiness_failure(ReadinessProgress::from_steps([
                ReadinessStep::GuestAuthenticated,
                ReadinessStep::GenerationAcknowledged,
                ReadinessStep::IdentityRepaired,
            ])));
        }
        if self.fail_at == Some(TestStage::NoopProbe) {
            return Err(readiness_failure(complete_readiness_progress()));
        }
        if self.fail_at == Some(TestStage::OutOfOrderReadyEvidence) {
            return ReadyAuthenticatedGuest::for_test(
                ReadinessProgress::from_steps([
                    ReadinessStep::GuestAuthenticated,
                    ReadinessStep::IdentityRepaired,
                ]),
                ExitStatus::Code(0),
            );
        }
        if self.fail_at == Some(TestStage::NonzeroReadyClaim) {
            return ReadyAuthenticatedGuest::for_test(
                complete_readiness_progress(),
                ExitStatus::Code(17),
            );
        }
        ReadyAuthenticatedGuest::for_test(complete_readiness_progress(), ExitStatus::Code(0))
    }

    fn execute(
        &mut self,
        _execute: &Execute,
        _guest: &mut ReadyAuthenticatedGuest,
    ) -> Result<PlatformExecution, PlatformFailure> {
        self.complete(TestStage::UserExecute)?;
        if self.fail_at == Some(TestStage::OversizedOutput) {
            return Ok(PlatformExecution::for_test(
                ExitStatus::Code(0),
                b"abcdefgh".to_vec(),
                b"WXYZ".to_vec(),
            ));
        }
        self.execution_count += 1;
        Ok(PlatformExecution::for_test(
            ExitStatus::Code(0),
            format!("execution-{}", self.execution_count).into_bytes(),
            Vec::new(),
        ))
    }

    fn stop(
        &mut self,
        _stop: &Stop,
        _guest: Option<&mut ReadyAuthenticatedGuest>,
    ) -> Result<PlatformStop, PlatformFailure> {
        if self.stop_failures_remaining > 0 {
            self.stop_failures_remaining -= 1;
            return Err(platform_failure());
        }
        self.complete(TestStage::Stop)?;
        Ok(PlatformStop::for_test(true, false))
    }

    fn rollback(&mut self, _launch: &Launch) -> Result<(), PlatformFailure> {
        if self.rollback_fails {
            Err(PlatformFailure::new(Recovery::RepairHost))
        } else {
            Ok(())
        }
    }
}

fn complete_readiness_progress() -> ReadinessProgress {
    ReadinessProgress::from_steps([
        ReadinessStep::GuestAuthenticated,
        ReadinessStep::GenerationAcknowledged,
        ReadinessStep::IdentityRepaired,
        ReadinessStep::NetworkRepaired,
    ])
}

const fn platform_failure() -> PlatformFailure {
    PlatformFailure::new(Recovery::ReplaceMachine)
}

fn readiness_failure(progress: ReadinessProgress) -> ReadinessFailure {
    ReadinessFailure::for_test(progress, Recovery::ReplaceMachine)
}
