//! Background replenishment retains one join handle per running construction, not one per
//! construction ever started.

#![cfg(unix)]

mod support;

use soma_hostd::Limits;
use support::{harness, intent, limits, op};

#[test]
fn repeated_replenishment_never_retains_a_finished_construction_thread() {
    let harness = harness(Limits {
        replenish_concurrency: 1,
        ..limits(1, 1)
    });
    let mut peak = 0;
    for round in 0..300_u32 {
        let _ = harness.pool.replenish();
        let mut spins = 0_u32;
        while harness.pool.occupancy().sterile == 0 {
            std::thread::yield_now();
            spins += 1;
            assert!(spins < 10_000_000, "round {round} never replenished");
        }
        let claim = harness
            .pool
            .claim(op(round), intent(round).fingerprint())
            .expect("claim");
        let evidence = harness
            .pool
            .transfer(claim.grant.expect("grant"), &intent(round))
            .expect("transfer");
        harness.pool.release(evidence.worker).expect("release");
        peak = peak.max(harness.pool.pending_replenishment());
    }
    assert!(
        peak <= 4,
        "300 replenishment cycles retained {peak} construction threads"
    );
    harness.pool.wait_replenishment();
    assert_eq!(harness.pool.pending_replenishment(), 0);
    assert_eq!(harness.pool.launcher().constructed(), 300);
}
