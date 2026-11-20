//! Group commit: a write is released only by a sync that covers it, and
//! concurrent writers share one sync.

use std::{
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use super::SyncBarrier;

/// A window long enough that every writer in a test has joined the batch before
/// the leader syncs, so the assertions are about the algorithm and not a race.
const TEST_WINDOW: Duration = Duration::from_millis(100);

#[test]
fn a_write_is_released_only_after_a_sync_that_covers_it() {
    let barrier = SyncBarrier::new(TEST_WINDOW);
    let syncs = Arc::new(AtomicUsize::new(0));
    let (entered, sync_entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let (done, finished) = mpsc::channel();

    let writer = {
        let syncs = Arc::clone(&syncs);
        thread::spawn(move || {
            let result = barrier.commit(|| {
                syncs.fetch_add(1, Ordering::SeqCst);
                let _entered = entered.send(());
                // Hold the sync open: the writer must not be released by a sync
                // that has not finished.
                let _held = released.recv();
                Ok(())
            });
            let _sent = done.send(result);
        })
    };

    // The writer is alone here, so it leads the batch and runs the sync itself.
    sync_entered
        .recv_timeout(Duration::from_secs(5))
        .expect("the leader must run its sync");
    assert_eq!(syncs.load(Ordering::SeqCst), 1, "one writer runs one sync");

    // The sync is still inside the closure, so `commit` cannot have returned: a
    // caller released before its own sync completed would treat a write as
    // durable that nothing had flushed.
    assert!(
        finished.recv_timeout(Duration::from_millis(200)).is_err(),
        "commit returned while its own sync was still running"
    );

    release.send(()).expect("the held sync must still be open");
    assert_eq!(
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("commit must return once its sync completes"),
        Ok(())
    );
    writer.join().expect("the writer thread must finish");
}

#[test]
fn concurrent_writers_share_one_sync() {
    const WRITERS: usize = 8;

    let barrier = Arc::new(SyncBarrier::new(TEST_WINDOW));
    let syncs = Arc::new(AtomicUsize::new(0));
    let all_ready = Arc::new(Barrier::new(WRITERS));
    let results = Arc::new(Mutex::new(Vec::new()));

    let writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let syncs = Arc::clone(&syncs);
            let all_ready = Arc::clone(&all_ready);
            let results = Arc::clone(&results);
            thread::spawn(move || {
                // Every writer has done its own file work and is about to commit,
                // so they all arrive inside the leader's window.
                all_ready.wait();
                let result = barrier.commit(|| {
                    syncs.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                });
                results
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(result);
            })
        })
        .collect();

    for writer in writers {
        writer.join().expect("a writer thread must finish");
    }

    let results = results
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(results.len(), WRITERS);
    assert!(
        results.iter().all(Result::is_ok),
        "every writer must be released successfully: {results:?}"
    );
    assert_eq!(
        syncs.load(Ordering::SeqCst),
        1,
        "{WRITERS} writers arriving together must share one sync"
    );
}

#[test]
fn a_writer_that_arrives_during_a_sync_gets_a_sync_of_its_own() {
    let barrier = Arc::new(SyncBarrier::new(TEST_WINDOW));
    let (entered, leader_in_sync) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let leader = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.commit(|| {
                let _entered = entered.send(());
                let _held = released.recv();
                Ok(())
            })
        })
    };
    leader_in_sync
        .recv_timeout(Duration::from_secs(5))
        .expect("the leader must run its sync");

    // The leader took its snapshot before it entered the sync, so a writer arriving now is not
    // covered by that sync and must not be released by it.
    let syncs = Arc::new(AtomicUsize::new(0));
    let (done, finished) = mpsc::channel();
    let late = {
        let barrier = Arc::clone(&barrier);
        let syncs = Arc::clone(&syncs);
        thread::spawn(move || {
            let result = barrier.commit(|| {
                syncs.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
            let _sent = done.send(result);
        })
    };
    assert!(
        finished.recv_timeout(Duration::from_millis(200)).is_err(),
        "a writer was released by a sync that started before it arrived"
    );

    release.send(()).expect("the held sync must still be open");
    assert_eq!(
        leader.join().expect("the leader thread must finish"),
        Ok(())
    );
    // No other writer comes, so nobody else will ever sync: the late writer must lead the next
    // sync itself instead of waiting for one forever.
    assert_eq!(
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("a writer that arrived during a sync must not wait for a sync nobody runs"),
        Ok(())
    );
    assert_eq!(
        syncs.load(Ordering::SeqCst),
        1,
        "the late writer runs the sync that covers it"
    );
    late.join().expect("the late writer thread must finish");
}
