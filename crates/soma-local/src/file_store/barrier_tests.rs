//! Group commit: a write is released only by a sync that covers it, and
//! concurrent writers share one sync.

use std::{
    sync::{
        Arc, Barrier, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use super::SyncBarrier;

/// A window long enough that every writer in a test has joined the batch before
/// the leader syncs, so the assertions are about the algorithm and not a race.
const TEST_WINDOW: Duration = Duration::from_millis(100);

/// How long a case waits for work another thread does before it fails.
const PATIENCE: Duration = Duration::from_secs(5);

/// Waits for another thread to make `condition` true, failing after [`PATIENCE`].
fn settle(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "{what} never happened");
        thread::sleep(Duration::from_millis(5));
    }
}

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
fn a_writer_that_arrives_after_the_leaders_snapshot_leads_the_next_sync_itself() {
    // No window, so the leader takes its snapshot at once and a second writer is
    // late for it by construction.
    let barrier = Arc::new(SyncBarrier::new(Duration::ZERO));
    let syncs = Arc::new(AtomicUsize::new(0));
    let (entered, leader_syncing) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();

    let leader = {
        let barrier = Arc::clone(&barrier);
        let syncs = Arc::clone(&syncs);
        thread::spawn(move || {
            barrier.commit(|| {
                syncs.fetch_add(1, Ordering::SeqCst);
                let _entered = entered.send(());
                let _held = released.recv();
                Ok(())
            })
        })
    };
    leader_syncing
        .recv_timeout(PATIENCE)
        .expect("the leader must run its sync");

    // The leader is inside its sync, so its snapshot is taken and cannot cover a
    // writer that arrives now.
    let (done, finished) = mpsc::channel();
    let _late = {
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
    settle("the late writer's arrival", || {
        barrier
            .batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .arrived
            == 2
    });

    // The leader's sync ends without covering the late writer, and no other
    // writer is coming to lead the next one.
    release.send(()).expect("the held sync must still be open");
    assert_eq!(leader.join().expect("the leader must finish"), Ok(()));
    assert_eq!(
        finished.recv_timeout(PATIENCE).expect(
            "a writer the last sync did not cover must run the next sync itself, \
             not wait for a writer that may never arrive"
        ),
        Ok(())
    );
    assert_eq!(
        syncs.load(Ordering::SeqCst),
        2,
        "the late write was made durable by a sync of its own"
    );
}
