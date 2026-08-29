use std::{
    fs::OpenOptions,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

use soma::{StateRevision, StateStore, StateStoreFailureKind};

use super::{FileStateStore, INSTANCE, TempRoot, instance, record};
use crate::file_store::{
    filesystem::ensure_directory,
    layout::{instance_lock_path, revision_path},
    revision::{commit_revision, read_record},
};

#[test]
fn concurrent_creates_have_exactly_one_winner() {
    let root = TempRoot::new("concurrent-create");
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for value in [b"first".as_slice(), b"second".as_slice()] {
        let barrier = Arc::clone(&barrier);
        let path = root.path().to_path_buf();
        let value = value.to_vec();
        handles.push(thread::spawn(move || {
            let mut store = FileStateStore::open(path).expect("open state store");
            barrier.wait();
            store.create(&instance(), record(&value))
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("worker did not panic"))
        .collect::<Vec<_>>();

    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome
                .as_ref()
                .is_err_and(|error| { error.kind() == StateStoreFailureKind::Conflict }))
            .count(),
        1
    );
}

#[test]
fn concurrent_revision_publication_never_replaces_an_existing_target() {
    let root = TempRoot::new("no-replace-publication");
    let directory = root.path().join(INSTANCE);
    ensure_directory(&directory).expect("create instance directory");
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for value in [b"first".as_slice(), b"second".as_slice()] {
        let barrier = Arc::clone(&barrier);
        let directory = directory.clone();
        let value = value.to_vec();
        handles.push(thread::spawn(move || {
            barrier.wait();
            commit_revision(&directory, StateRevision::INITIAL, &record(&value))
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("publisher did not panic"))
        .collect::<Vec<_>>();

    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome
                .as_ref()
                .is_err_and(|error| error.kind() == StateStoreFailureKind::Conflict))
            .count(),
        1
    );
    let published = read_record(&revision_path(
        root.path(),
        &instance(),
        StateRevision::INITIAL,
    ))
    .expect("read winning publication");
    assert!(matches!(published.as_bytes(), b"first" | b"second"));
}

#[test]
fn an_exclusive_lock_blocks_other_store_handles() {
    let root = TempRoot::new("lock-contention");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"committed"))
        .expect("create state");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(instance_lock_path(root.path(), &instance()))
        .expect("open lock");
    lock.lock().expect("hold exclusive lock");
    let path = root.path().to_path_buf();
    let (started_tx, started_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut contender = FileStateStore::open(path).expect("open contender");
        started_tx.send(()).expect("announce contender");
        let result = contender.load(&instance());
        finished_tx.send(result).expect("return contender result");
    });
    started_rx.recv().expect("contender started");

    assert!(
        finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "contender must wait for the exclusive lock"
    );
    lock.unlock().expect("release exclusive lock");
    let stored = finished_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("contender unblocked")
        .expect("load succeeds")
        .expect("state exists");
    assert_eq!(stored.record().as_bytes(), b"committed");
    worker.join().expect("contender did not panic");
}
