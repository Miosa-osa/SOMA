use std::{
    fs,
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use soma::StateStore;

use super::{FileStateStore, TempRoot, instance};
use crate::file_store::{layout::LOCK_DIRECTORY, turn::turn_path};

/// SOMA#243: a second caller, here through its own store as a second connection would be, waits
/// for the turn and gets it once the first gives it back.
#[test]
fn a_second_turn_waits_for_the_first() {
    let root = TempRoot::new("turn-wait");
    let first = FileStateStore::open(root.path()).expect("open first store");
    let held = first.turn(&instance()).expect("first turn");
    let path = root.path().to_path_buf();
    let second = thread::spawn(move || {
        let store = FileStateStore::open(path).expect("open second store");
        store.turn(&instance()).map(drop)
    });
    thread::sleep(Duration::from_millis(200));
    assert!(!second.is_finished(), "the second turn did not wait");
    drop(held);
    second
        .join()
        .expect("second caller joins")
        .expect("second turn");
}

/// However many callers contend, one holds the turn at a time, and none leaves a file behind.
#[test]
fn contended_turns_are_exclusive_and_leave_nothing() {
    const CALLERS: usize = 8;
    const ROUNDS: usize = 50;
    let root = TempRoot::new("turn-contended");
    let holders = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(CALLERS));
    let callers = (0..CALLERS)
        .map(|_| {
            let (path, holders, barrier) = (
                root.path().to_path_buf(),
                Arc::clone(&holders),
                Arc::clone(&barrier),
            );
            thread::spawn(move || {
                let store = FileStateStore::open(path).expect("open store");
                barrier.wait();
                for _ in 0..ROUNDS {
                    let turn = store.turn(&instance()).expect("turn");
                    assert_eq!(holders.fetch_add(1, Ordering::SeqCst), 0, "two holders");
                    thread::yield_now();
                    holders.fetch_sub(1, Ordering::SeqCst);
                    drop(turn);
                }
            })
        })
        .collect::<Vec<_>>();
    for caller in callers {
        caller.join().expect("caller joins");
    }
    let path = turn_path(&root.path().join(LOCK_DIRECTORY), &instance());
    assert!(
        cfg!(not(unix)) || !path.exists(),
        "a turn file outlived every holder"
    );
}

/// A holder that died left its file: the next caller takes the turn at once and removes it.
#[test]
fn a_turn_file_a_dead_holder_left_is_taken_and_removed() {
    let root = TempRoot::new("turn-left");
    let store = FileStateStore::open(root.path()).expect("open store");
    let path = turn_path(&root.path().join(LOCK_DIRECTORY), &instance());
    fs::write(&path, b"").expect("leave a turn file");
    drop(store.turn(&instance()).expect("turn"));
    assert!(cfg!(not(unix)) || !path.exists());
}
