//! The explicit on-demand exhausted behavior: a worker built inline is usable.

#![cfg(unix)]

mod support;

use std::time::Duration;

use soma_hostd::{ClaimClass, ExhaustedBehavior, Limits, Phase, testing::FaultPlan};
use support::{harness, intent, limits, op};

#[test]
fn an_inline_worker_is_labelled_on_demand_and_transfers_after_a_slow_construction() {
    let harness = harness(Limits {
        target: 0,
        claim_deadline: Duration::from_millis(20),
        construction_deadline: Duration::from_secs(2),
        exhausted: ExhaustedBehavior::ConstructInline,
        ..limits(0, 1)
    });
    harness.pool.launcher().set_plan(FaultPlan {
        construct_delay: Duration::from_millis(60),
        ..FaultPlan::default()
    });
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    assert_eq!(claim.outcome.class, ClaimClass::OnDemand);
    assert!(
        claim
            .grant
            .as_ref()
            .expect("grant")
            .won_at()
            .elapsed()
            .lt(&harness.pool.limits().claim_deadline),
        "the claim deadline is measured from the win, not from the arrival of the request"
    );
    let evidence = harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect("an inline worker transfers inside its own claim deadline");
    harness.pool.start(evidence.worker).expect("start");
    assert_eq!(
        harness.pool.inspect(evidence.worker).map(|view| view.phase),
        Some(Phase::Running)
    );
    assert_eq!(
        harness
            .pool
            .claim(op(1), intent(1).fingerprint())
            .expect("replay")
            .outcome
            .class,
        ClaimClass::OnDemand
    );
    harness.pool.release(evidence.worker).expect("release");
}
