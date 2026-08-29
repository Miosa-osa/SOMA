//! Start and release contend on one owned worker without ever losing it.

#![cfg(unix)]

mod support;

use std::sync::{Arc, Barrier};

use soma_hostd::{LifecycleError, Phase};
use support::{harness, intent, limits, op};

#[test]
fn a_release_racing_a_start_is_refused_by_phase_and_never_told_the_pool_owns_nothing() {
    let harness = harness(limits(1, 2));
    let mut refused = 0_u32;
    let mut released_first = 0_u32;
    for round in 0..400_u32 {
        harness.pool.replenish_blocking().expect("replenish");
        let worker = {
            let claim = harness
                .pool
                .claim(op(round), intent(round).fingerprint())
                .expect("claim");
            let worker = claim.outcome.worker;
            harness
                .pool
                .transfer(claim.grant.expect("grant"), &intent(round))
                .expect("transfer");
            worker
        };
        let barrier = Arc::new(Barrier::new(2));
        let starter = {
            let pool = Arc::clone(&harness.pool);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                pool.start(worker)
            })
        };
        let releaser = {
            let pool = Arc::clone(&harness.pool);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                pool.release(worker)
            })
        };
        let start = starter.join().expect("start thread");
        let release = releaser.join().expect("release thread");
        match (&start, &release) {
            (Ok(()), Ok(_)) => {}
            (Ok(()), Err(LifecycleError::Phase { phase, .. })) => {
                assert_eq!(*phase, Phase::Assigned);
                refused += 1;
                harness
                    .pool
                    .release(worker)
                    .expect("the pool still owns the started worker");
            }
            (Err(LifecycleError::Unknown(_)), Ok(_)) => released_first += 1,
            other => panic!("round {round}: unexpected {other:?}"),
        }
        assert_eq!(
            harness.pool.inspect(worker).map(|view| view.phase),
            Some(Phase::Dead),
            "round {round} left the worker owned"
        );
        assert!(
            harness.pool.release(worker).is_err(),
            "round {round} released the worker twice"
        );
    }
    eprintln!(
        "start against release over 400 rounds: {refused} releases refused mid-start, \
         {released_first} releases won the race"
    );
    assert_eq!(harness.admission.usage().residents, 0);
}
