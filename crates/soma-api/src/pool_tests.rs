use std::{
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use super::FacadePool;

#[test]
fn every_facade_is_opened_before_the_pool_is_published() {
    let mut opened = 0;
    let pool = FacadePool::open(4, || {
        opened += 1;
        Ok::<_, ()>(opened)
    })
    .expect("open the pool");

    assert_eq!(opened, 4);
    assert!(
        *pool
            .acquire_timeout(Duration::from_millis(10))
            .expect("one facade is available")
            > 0
    );
}

#[test]
fn concurrent_callers_receive_distinct_facades() {
    let pool = FacadePool::open(8, || Ok::<_, ()>(())).expect("open the pool");
    let gate = Arc::new(Barrier::new(8));
    let mut callers = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let gate = Arc::clone(&gate);
        callers.push(thread::spawn(move || {
            let _lease = pool
                .acquire_timeout(Duration::from_millis(10))
                .expect("one facade per caller");
            gate.wait();
        }));
    }
    for caller in callers {
        caller.join().expect("caller completed");
    }
}

#[test]
fn bounded_acquisition_times_out_and_a_returned_lease_is_reused() {
    let pool = FacadePool::open(1, || Ok::<_, ()>(7)).expect("open the pool");
    let held = pool
        .acquire_timeout(Duration::from_millis(10))
        .expect("initial facade");
    assert!(pool.acquire_timeout(Duration::ZERO).is_none());
    drop(held);
    assert_eq!(
        *pool
            .acquire_timeout(Duration::from_millis(10))
            .expect("returned facade is available"),
        7
    );
}
