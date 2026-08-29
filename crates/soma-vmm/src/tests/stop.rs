use crate::{CleanupEvidence, FailureKind, FailurePhase, Machine, Milestone, Recovery};

use super::{
    fixtures::{launch, stop},
    platform::{DeterministicPlatform, TestStage},
};

#[test]
fn stop_after_ready_returns_complete_cleanup_and_replays_idempotently() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");
    let request = stop([5; 16]);

    let first = machine.stop(request.clone()).expect("stop cleans up");
    let replay = machine
        .stop(request)
        .expect("exact stop replay is idempotent");

    assert_eq!(replay, first);
    assert_eq!(first.cleanup(), CleanupEvidence::Complete);
    assert_eq!(
        first.milestones(),
        &[
            Milestone::StopRequested,
            Milestone::CleanupCompleted,
            Milestone::Stopped,
        ]
    );
}

#[test]
fn stop_failure_reports_incomplete_cleanup_and_replays() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_at(TestStage::Stop));
    machine.launch(launch()).expect("launch is ready");
    let request = stop([6; 16]);

    let first = machine
        .stop(request.clone())
        .expect_err("failed stop cannot claim cleanup");
    let replay = machine
        .stop(request)
        .expect_err("the stop failure receipt is idempotently replayed");

    assert_eq!(replay, first);
    assert_eq!(first.kind(), FailureKind::StopFailed);
    assert_eq!(first.phase(), FailurePhase::Stop);
    assert_eq!(first.recovery(), Recovery::ReplaceMachine);
    assert_eq!(first.cleanup(), CleanupEvidence::Incomplete);
    assert_eq!(first.milestones(), &[Milestone::StopRequested]);
}
