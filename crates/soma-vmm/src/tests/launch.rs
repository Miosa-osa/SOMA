use crate::{CleanupEvidence, FailureKind, FailurePhase, Machine, Milestone, Recovery};

use super::{
    fixtures::{launch, machine_spec},
    platform::{DeterministicPlatform, TestStage},
};

#[test]
fn launch_reaches_ready_only_after_the_required_pipeline() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());

    let ready = machine.launch(launch()).expect("healthy launch is ready");

    assert_eq!(
        ready.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::GuestAuthenticated,
            Milestone::GenerationAcknowledged,
            Milestone::IdentityRepaired,
            Milestone::NetworkRepaired,
            Milestone::FirstCommandSucceeded,
            Milestone::Ready,
        ]
    );
    assert_eq!(ready.machine(), machine_spec());
}

#[test]
fn out_of_order_fused_progress_fails_closed_at_the_first_missing_stage() {
    let mut machine = Machine::with_platform(DeterministicPlatform::out_of_order_ready_evidence());

    let failure = machine
        .launch(launch())
        .expect_err("out-of-order guest progress cannot authorize readiness");

    assert_eq!(failure.kind(), FailureKind::GenerationAcknowledgementFailed);
    assert_eq!(failure.phase(), FailurePhase::GenerationAcknowledgement);
    assert_eq!(failure.recovery(), Recovery::RepairHost);
    assert_eq!(failure.cleanup(), CleanupEvidence::Complete);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::GuestAuthenticated,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn nonzero_readiness_result_cannot_claim_first_command_or_ready() {
    let mut machine = Machine::with_platform(DeterministicPlatform::nonzero_ready_claim());

    let failure = machine
        .launch(launch())
        .expect_err("a nonzero readiness command cannot authorize the guest");

    assert_eq!(failure.kind(), FailureKind::ReadinessProbeFailed);
    assert_eq!(failure.phase(), FailurePhase::ReadinessProbe);
    assert_eq!(failure.recovery(), Recovery::ReplaceMachine);
    assert_eq!(failure.cleanup(), CleanupEvidence::Complete);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::GuestAuthenticated,
            Milestone::GenerationAcknowledged,
            Milestone::IdentityRepaired,
            Milestone::NetworkRepaired,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn out_of_order_restore_progress_cannot_mint_restored_type_state() {
    let mut machine =
        Machine::with_platform(DeterministicPlatform::out_of_order_restore_evidence());

    let failure = machine
        .launch(launch())
        .expect_err("restore evidence cannot skip artifact verification");

    assert_eq!(failure.kind(), FailureKind::GenerationVerificationFailed);
    assert_eq!(failure.phase(), FailurePhase::ArtifactVerification);
    assert_eq!(failure.recovery(), Recovery::RepairHost);
    assert_eq!(failure.cleanup(), CleanupEvidence::Complete);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn restore_failure_preserves_verified_artifact_evidence() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_at(TestStage::Restore));

    let failure = machine
        .launch(launch())
        .expect_err("restore failure must preserve only completed verification evidence");

    assert_eq!(failure.kind(), FailureKind::RestoreFailed);
    assert_eq!(failure.phase(), FailurePhase::Restore);
    assert_eq!(failure.recovery(), Recovery::ReplaceMachine);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn guest_authentication_failure_preserves_restored_machine_evidence() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_at(
        TestStage::GuestAuthentication,
    ));

    let failure = machine
        .launch(launch())
        .expect_err("guest authentication must gate all repair authority");

    assert_eq!(failure.kind(), FailureKind::GuestAuthenticationFailed);
    assert_eq!(failure.phase(), FailurePhase::GuestAuthentication);
    assert_eq!(failure.recovery(), Recovery::ReplaceMachine);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn launch_failure_reports_completed_milestones_and_recovery() {
    let mut machine =
        Machine::with_platform(DeterministicPlatform::failing_at(TestStage::IdentityRepair));

    let failure = machine
        .launch(launch())
        .expect_err("identity repair must gate readiness");

    assert_eq!(failure.kind(), FailureKind::IdentityRepairFailed);
    assert_eq!(failure.phase(), FailurePhase::IdentityRepair);
    assert_eq!(failure.recovery(), Recovery::ReplaceMachine);
    assert_eq!(failure.cleanup(), CleanupEvidence::Complete);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::GuestAuthenticated,
            Milestone::GenerationAcknowledged,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn generation_acknowledgement_requires_distinct_platform_evidence() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_at(
        TestStage::GenerationAcknowledgement,
    ));

    let failure = machine
        .launch(launch())
        .expect_err("generation acknowledgement must gate repair authority");

    assert_eq!(failure.kind(), FailureKind::GenerationAcknowledgementFailed);
    assert_eq!(failure.phase(), FailurePhase::GenerationAcknowledgement);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::GuestAuthenticated,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn network_repair_requires_distinct_platform_evidence() {
    let mut machine =
        Machine::with_platform(DeterministicPlatform::failing_at(TestStage::NetworkRepair));

    let failure = machine
        .launch(launch())
        .expect_err("network repair must gate command readiness");

    assert_eq!(failure.kind(), FailureKind::NetworkRepairFailed);
    assert_eq!(failure.phase(), FailurePhase::NetworkRepair);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::ArtifactsVerified,
            Milestone::MachineRestored,
            Milestone::GuestAuthenticated,
            Milestone::GenerationAcknowledged,
            Milestone::IdentityRepaired,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}

#[test]
fn launch_reports_incomplete_cleanup_when_rollback_fails() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_with_rollback(
        TestStage::NoopProbe,
    ));

    let failure = machine
        .launch(launch())
        .expect_err("failed cleanup cannot be hidden");

    assert_eq!(failure.kind(), FailureKind::ReadinessProbeFailed);
    assert_eq!(failure.recovery(), Recovery::RepairHost);
    assert_eq!(failure.cleanup(), CleanupEvidence::Incomplete);
    assert_eq!(
        failure.milestones().last(),
        Some(&Milestone::RollbackStarted)
    );
}

#[test]
fn byte_equivalent_launch_replay_returns_the_original_receipt() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    let request = launch();
    let first = machine
        .launch(request.clone())
        .expect("first launch is ready");

    let replay = machine.launch(request).expect("exact replay is idempotent");

    assert_eq!(replay, first);
}

#[test]
fn public_machine_is_constructible_and_fails_closed_without_a_platform() {
    let mut machine = Machine::new();

    let failure = machine
        .launch(launch())
        .expect_err("the library must not pretend to provide a production platform");

    assert_eq!(failure.kind(), FailureKind::GenerationVerificationFailed);
    assert_eq!(failure.phase(), FailurePhase::ArtifactVerification);
    assert_eq!(failure.recovery(), Recovery::RepairHost);
    assert_eq!(failure.cleanup(), CleanupEvidence::Complete);
    assert_eq!(
        failure.milestones(),
        &[
            Milestone::RequestAccepted,
            Milestone::RollbackStarted,
            Milestone::CleanupCompleted,
        ]
    );
}
