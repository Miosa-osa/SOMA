//! Claim latency over 1,000 claims with the durable ledger; printed, not asserted.

#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use soma_hostd::Limits;
use support::{harness, intent, limits, op};

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    let rank = (sorted.len() * percent).div_ceil(100).max(1) - 1;
    sorted[rank.min(sorted.len() - 1)]
}

#[test]
fn claim_latency_over_1000_claims_is_recorded() {
    let harness = harness(Limits {
        replenish_concurrency: 8,
        binding_limit: 4096,
        ..limits(1000, 1000)
    });
    let prepared = Instant::now();
    harness.pool.replenish_blocking().expect("replenish");
    let prepared = prepared.elapsed();
    let mut claim = Vec::with_capacity(1000);
    let mut transfer = Vec::with_capacity(1000);
    for index in 0..1000_u32 {
        let started = Instant::now();
        let outcome = harness
            .pool
            .claim(op(index), intent(index).fingerprint())
            .expect("claim");
        claim.push(started.elapsed());
        let started = Instant::now();
        harness
            .pool
            .transfer(outcome.grant.expect("grant"), &intent(index))
            .expect("transfer");
        transfer.push(started.elapsed());
    }
    claim.sort_unstable();
    transfer.sort_unstable();
    let ledger = harness.pool.ledger().root().display().to_string();
    let records = harness.pool.ledger().records().expect("records").len();
    eprintln!(
        "claim latency over 1000 claims (one durable ledger record with file and directory fsync per claim, ledger at {ledger}): p50 {:?}, p99 {:?}, max {:?}",
        percentile(&claim, 50),
        percentile(&claim, 99),
        claim[claim.len() - 1]
    );
    eprintln!(
        "transfer latency over 1000 transfers (nine durable records each, in-process launcher): p50 {:?}, p99 {:?}, max {:?}",
        percentile(&transfer, 50),
        percentile(&transfer, 99),
        transfer[transfer.len() - 1]
    );
    eprintln!("preparation of 1000 workers took {prepared:?}; the ledger holds {records} records");
    assert_eq!(harness.pool.occupancy().assigned, 1000);
}
