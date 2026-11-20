//! Releasing a record (ADR 0055) and what a release, or a failed create, leaves on disk.

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
};

use soma::{InstanceId, StateRevision, StateStore, StateStoreFailureKind};

use super::{FileStateStore, INSTANCE, TempRoot, instance, record};
use crate::file_store::layout::{LOCK_DIRECTORY, revision_path};

fn revision(value: u64) -> StateRevision {
    StateRevision::new(value).expect("revision")
}

/// The record the store holds after a create and two replacements, at revision 3.
fn terminal_record(store: &mut FileStateStore, instance_id: &InstanceId) -> StateRevision {
    store
        .create(instance_id, record(b"launching"))
        .expect("create");
    store
        .compare_exchange(instance_id, StateRevision::INITIAL, record(b"active"))
        .expect("activate");
    store
        .compare_exchange(instance_id, revision(2), record(b"terminal"))
        .expect("terminate")
}

#[test]
fn a_release_removes_every_revision_and_the_instance_directory() {
    let root = TempRoot::new("release");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    let current = terminal_record(&mut store, &instance());

    store.release(&instance(), current).expect("release");

    assert!(!root.path().join(INSTANCE).exists());
    assert!(store.load(&instance()).expect("load").is_none());
    assert!(store.list().expect("list").is_empty());
    assert_eq!(
        store
            .create(&instance(), record(b"again"))
            .expect("a released identity may be created again"),
        StateRevision::INITIAL
    );
}

#[test]
fn a_release_of_a_revision_that_is_not_current_or_of_nothing_conflicts_and_keeps_everything() {
    let root = TempRoot::new("release-stale");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    let current = terminal_record(&mut store, &instance());

    let stale = store
        .release(&instance(), revision(2))
        .expect_err("a stale revision is not released");
    assert_eq!(stale.kind(), StateStoreFailureKind::Conflict);
    let kept = store.load(&instance()).expect("load").expect("kept");
    assert_eq!(kept.revision(), current);
    assert_eq!(kept.record().as_bytes(), b"terminal");

    let other = InstanceId::new("abcdefabcdefabcdefabcdefabcdef01").expect("instance");
    let absent = store
        .release(&other, StateRevision::INITIAL)
        .expect_err("nothing to release");
    assert_eq!(absent.kind(), StateStoreFailureKind::Conflict);
    assert!(!root.path().join(other.as_str()).exists());
}

#[test]
fn a_release_removes_a_superseded_revision_a_crash_left_behind() {
    let root = TempRoot::new("release-superseded");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    let current = terminal_record(&mut store, &instance());
    // A crash between a replacement and its prune leaves the older revision beside the new one.
    fs::write(
        revision_path(root.path(), &instance(), revision(2)),
        b"active",
    )
    .expect("inject the revision a prune would have removed");

    store.release(&instance(), current).expect("release");

    assert!(!root.path().join(INSTANCE).exists());
}

#[test]
fn a_release_never_removes_a_record_the_store_cannot_read() {
    let root = TempRoot::new("release-corrupt");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"terminal"))
        .expect("create");
    let path = revision_path(root.path(), &instance(), StateRevision::INITIAL);
    fs::write(&path, b"").expect("truncate the current revision");

    let error = store
        .release(&instance(), StateRevision::INITIAL)
        .expect_err("an unreadable record is not released");

    assert_eq!(error.kind(), StateStoreFailureKind::Corrupt);
    assert!(path.exists());
}

#[test]
fn records_an_earlier_process_left_are_released_after_a_restart() {
    let root = TempRoot::new("release-restart");
    let mut first = FileStateStore::open(root.path()).expect("open state store");
    let current = terminal_record(&mut first, &instance());
    drop(first);

    let mut restarted = FileStateStore::open(root.path()).expect("reopen state store");
    assert_eq!(restarted.list().expect("list"), [instance()]);
    restarted.release(&instance(), current).expect("release");

    assert!(restarted.list().expect("list").is_empty());
}

/// What a create that failed on a full store left on host-04: a directory naming an Instance,
/// holding only an empty temporary file.
#[test]
fn a_directory_with_no_record_is_removed_the_first_time_it_is_read() {
    let root = TempRoot::new("leftover");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    let leftover = root.path().join(INSTANCE);
    fs::create_dir(&leftover).expect("leftover directory");
    fs::write(leftover.join(".tmp-0000004242-00000000000000000007"), b"")
        .expect("the empty temporary file a failed write left");
    assert_eq!(store.list().expect("list"), [instance()]);

    assert!(store.load(&instance()).expect("load").is_none());

    assert!(!leftover.exists());
    assert!(store.list().expect("list").is_empty());
}

/// SOMA#127: every create and destroy left its terminal record, and the state root filled. Ten
/// thousand create, replace, and release cycles must leave nothing but the shard locks.
///
/// They run as eight threads, each on its own root. A root's stores share one sync barrier, and
/// a writer that arrives after a sync took its snapshot waits for the next writer to lead
/// (SOMA#118), which the last writer on a shared root may never get, so the roots are separate.
#[test]
fn ten_thousand_create_and_release_cycles_leave_only_the_shard_locks() {
    const ROOTS: u32 = 8;
    let (base, cycles) = cycle_base();
    let per_root = cycles / ROOTS;
    let workers = (0..ROOTS)
        .map(|root_index| {
            let root = base.path().join(format!("root-{root_index}"));
            thread::spawn(move || {
                let mut store = FileStateStore::open(&root).expect("open state store");
                for cycle in 1..=per_root {
                    let id = InstanceId::new(format!("{cycle:032x}")).expect("instance");
                    let current = cycle_once(&mut store, &id);
                    store.release(&id, current).expect("release");
                }
                root
            })
        })
        .collect::<Vec<_>>();
    // Every worker is joined before any is judged, so the base outlives all of them.
    let joined = workers
        .into_iter()
        .map(thread::JoinHandle::join)
        .collect::<Vec<_>>();
    for worker in joined {
        let root = worker.expect("worker");
        let (directories, bytes) = footprint(&root);
        assert_eq!(
            directories, 0,
            "{per_root} cycles left Instance directories"
        );
        assert_eq!(bytes, 0, "{per_root} cycles left record bytes");
        let locks = fs::read_dir(root.join(LOCK_DIRECTORY))
            .expect("lock directory")
            .count();
        assert!(locks <= 256, "{locks} shard locks");
    }
}

/// The production state root is a tmpfs, and a tmpfs is also what makes ten thousand rounds of
/// synced writes quick enough for a unit test. Where there is none the run is shorter, and says so.
fn cycle_base() -> (TempRoot, u32) {
    let shared_memory = Path::new("/dev/shm");
    if shared_memory.is_dir() {
        let base = TempRoot::under(shared_memory, "release-cycles");
        if fs::create_dir(base.path()).is_ok() {
            return (base, 10_000);
        }
    }
    eprintln!("no writable tmpfs at /dev/shm: running 400 cycles on the temporary directory");
    (TempRoot::new("release-cycles"), 400)
}

/// One sandbox's writes, sized like host-04's records (a terminal one was 5.4 KB).
fn cycle_once(store: &mut FileStateStore, id: &InstanceId) -> StateRevision {
    let sized = |bytes: usize| record(&vec![b'x'; bytes]);
    store.create(id, sized(2_000)).expect("launching");
    let active = store
        .compare_exchange(id, StateRevision::INITIAL, sized(4_000))
        .expect("active");
    let terminating = store
        .compare_exchange(id, active, sized(4_500))
        .expect("terminating");
    store
        .compare_exchange(id, terminating, sized(5_400))
        .expect("terminal")
}

/// The Instance directories under `root` and the bytes of every file anywhere beneath it.
fn footprint(root: &Path) -> (usize, u64) {
    let mut directories = 0;
    let mut bytes = 0;
    let mut pending: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("read directory") {
            let entry = entry.expect("entry");
            let metadata = entry.metadata().expect("metadata");
            if metadata.is_dir() {
                if directory == root && entry.file_name() != LOCK_DIRECTORY {
                    directories += 1;
                }
                pending.push(entry.path());
            } else {
                bytes += metadata.len();
            }
        }
    }
    (directories, bytes)
}
