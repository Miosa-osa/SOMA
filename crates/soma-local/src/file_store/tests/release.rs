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
/// thousand create, replace, and release cycles, from eight stores sharing one root as a facade
/// pool does, must leave nothing but the shard locks.
#[test]
fn ten_thousand_create_and_release_cycles_leave_only_the_shard_locks() {
    const STORES: u32 = 8;
    let (root, cycles) = cycle_root();
    let per_store = cycles / STORES;
    let workers = (0..STORES)
        .map(|store_index| {
            let path = root.path().to_path_buf();
            thread::spawn(move || {
                let mut store = FileStateStore::open(&path).expect("open state store");
                for cycle in 0..per_store {
                    // Each store gets its own shard, so the stores contend only on the sync.
                    let shard = 0x10 * (store_index + 1);
                    let id = InstanceId::new(format!("{shard:02x}{cycle:030x}")).expect("instance");
                    let current = cycle_once(&mut store, &id);
                    store.release(&id, current).expect("release");
                }
            })
        })
        .collect::<Vec<_>>();
    // Every worker is joined before any is judged, so the root outlives all of them.
    let joined = workers
        .into_iter()
        .map(thread::JoinHandle::join)
        .collect::<Vec<_>>();
    for worker in joined {
        worker.expect("worker");
    }

    let (directories, bytes) = footprint(root.path());
    assert_eq!(directories, 0, "{cycles} cycles left Instance directories");
    assert_eq!(bytes, 0, "{cycles} cycles left record bytes");
    let locks = fs::read_dir(root.path().join(LOCK_DIRECTORY))
        .expect("lock directory")
        .count();
    assert!(locks <= 256, "{locks} shard locks");
}

/// The production state root is a tmpfs, and a tmpfs is also what makes ten thousand rounds of
/// synced writes quick enough for a unit test. Where there is none the run is shorter, and says so.
fn cycle_root() -> (TempRoot, u32) {
    let shared_memory = Path::new("/dev/shm");
    if shared_memory.is_dir() {
        let root = TempRoot::under(shared_memory, "release-cycles");
        if fs::create_dir(root.path()).is_ok() {
            return (root, 10_000);
        }
    }
    eprintln!("no writable tmpfs at /dev/shm: running 800 cycles on the temporary directory");
    (TempRoot::new("release-cycles"), 800)
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
