//! A state root that cannot take another byte (SOMA#127).
//!
//! host-04's 2G tmpfs filled, and from then on every create failed its first write after the
//! Instance's directory and a temporary file already existed, leaving both behind. A child
//! process stands in for the full filesystem: started with a file size limit of zero and
//! `SIGXFSZ` ignored, its every write to a file fails with an error, as a write to a full tmpfs
//! does, while removing a name still succeeds.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use soma::{InstanceId, StateRecord, StateRevision, StateStore, StateStoreFailureKind};
use soma_local::FileStateStore;

const CHILD_ROOT: &str = "SOMA_LOCAL_FULL_STORE_ROOT";
const KEPT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REFUSED: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn on_a_full_store_a_create_leaves_nothing_behind_and_a_release_still_frees_space() {
    let base = tempfile::tempdir().expect("temporary directory");
    let root = base.path().join("state");
    let mut store = FileStateStore::open(&root).expect("open state store");
    store
        .create(&id(KEPT), record(b"terminal"))
        .expect("the record the child releases");
    drop(store);

    let output = Command::new("sh")
        .args(["-c", "trap '' XFSZ; ulimit -f 0; exec \"$@\"", "sh"])
        .arg(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "child_on_a_full_store", "--nocapture"])
        .env(CHILD_ROOT, &root)
        .output()
        .expect("run the child on a full store");

    let said = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "child failed: {said}");
    assert!(
        said.contains("outcome: create unavailable, release ok"),
        "child said: {said}"
    );
    assert_eq!(entries(&root), [".locks"], "only the shard locks remain");
}

/// Runs only as the child of the test above, under its file size limit.
#[test]
fn child_on_a_full_store() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    let mut store = FileStateStore::open(PathBuf::from(root)).expect("open state store");
    let create = store
        .create(&id(REFUSED), record(&[b'x'; 5_400]))
        .expect_err("no byte can be written");
    assert_eq!(create.kind(), StateStoreFailureKind::Unavailable);
    store
        .release(&id(KEPT), StateRevision::INITIAL)
        .expect("a release writes nothing, so a full store can still free a record");
    // Standard output is a pipe, which the file size limit does not reach.
    println!("outcome: create unavailable, release ok");
}

fn id(value: &str) -> InstanceId {
    InstanceId::new(value).expect("instance")
}

fn record(bytes: &[u8]) -> StateRecord {
    StateRecord::from_bytes(bytes.to_vec()).expect("record")
}

fn entries(root: &Path) -> Vec<String> {
    let mut names = fs::read_dir(root)
        .expect("state root")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}
