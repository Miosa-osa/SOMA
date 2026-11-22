//! The suspension store end to end on real files: seal, open, expiry, damage and destroy.

use std::fs;
use std::path::Path;

use super::store::{Store, StoreError};
use super::{Header, MemberRecord, Shape};
use crate::ids::SuspensionId;
use crate::{GenerationId, HostProfileDigest};

fn header(id: u8) -> Header {
    Header {
        id: SuspensionId::new([id; 16]).expect("id"),
        generation: GenerationId::new([2; 32]).expect("generation"),
        host_profile: HostProfileDigest::new([3; 32]).expect("profile"),
        tenant: "tenant-a".to_owned(),
        shape: Shape {
            vcpus: 2,
            memory_mib: 4096,
            storage_mib: 20480,
        },
        created_unix: 1_000,
        expires_unix: 2_000,
        members: [MemberRecord {
            bytes: 0,
            sha256: [0; 32],
        }; 3],
    }
}

fn stage(directory: &Path, overlay: bool) {
    fs::create_dir_all(directory).expect("staging");
    fs::write(directory.join("memory.raw"), vec![7_u8; 3 << 20]).expect("memory");
    if overlay {
        fs::write(directory.join("overlay.raw"), b"overlay bytes").expect("overlay");
    }
    fs::write(directory.join("state.somasnap"), b"state bytes").expect("state");
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    staged: std::path::PathBuf,
    resumed: std::path::PathBuf,
    root: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("dir");
    let root = dir.path().join("store");
    let store = Store::new(&root, dir.path().join("master.key"));
    Fixture {
        staged: dir.path().join("staged"),
        resumed: dir.path().join("resumed"),
        root,
        store,
        _dir: dir,
    }
}

#[test]
fn a_sealed_suspension_opens_to_exactly_what_was_staged_and_the_plaintext_is_gone() {
    let f = fixture();
    stage(&f.staged, true);
    let sealed = f.store.seal(&f.staged, header(1)).expect("seal");
    assert_eq!(sealed.members[0].bytes, 3 << 20);
    for name in ["memory.raw", "overlay.raw", "state.somasnap"] {
        assert!(
            !f.staged.join(name).exists(),
            "{name} plaintext left behind"
        );
    }
    // Nothing in the store is plaintext.
    let memory = fs::read(f.root.join(sealed.id.hex()).join("memory.sealed")).expect("sealed");
    assert!(!memory.windows(64).any(|w| w.iter().all(|b| *b == 7)));

    let opened = f.store.open(sealed.id, 1_500, &f.resumed).expect("open");
    assert_eq!(opened, sealed);
    assert_eq!(
        fs::read(f.resumed.join("memory.raw")).expect("m"),
        vec![7_u8; 3 << 20]
    );
    assert_eq!(
        fs::read(f.resumed.join("overlay.raw")).expect("o"),
        b"overlay bytes"
    );
    assert_eq!(
        fs::read(f.resumed.join("state.somasnap")).expect("s"),
        b"state bytes"
    );
}

#[test]
fn a_generation_without_writable_storage_has_an_empty_overlay_and_none_is_staged_back() {
    let f = fixture();
    stage(&f.staged, false);
    let sealed = f.store.seal(&f.staged, header(2)).expect("seal");
    assert_eq!(sealed.members[1].bytes, 0);
    f.store.open(sealed.id, 1_500, &f.resumed).expect("open");
    assert!(!f.resumed.join("overlay.raw").exists());
}

#[test]
fn an_expired_suspension_is_destroyed_instead_of_opened() {
    let f = fixture();
    stage(&f.staged, true);
    let sealed = f.store.seal(&f.staged, header(3)).expect("seal");
    assert!(matches!(
        f.store.open(sealed.id, 2_000, &f.resumed),
        Err(StoreError::Expired)
    ));
    assert!(matches!(
        f.store.header(sealed.id),
        Err(StoreError::Missing)
    ));
}

#[test]
fn a_damaged_suspension_is_destroyed_rather_than_resumed() {
    let f = fixture();
    stage(&f.staged, true);
    let sealed = f.store.seal(&f.staged, header(4)).expect("seal");
    let path = f.root.join(sealed.id.hex()).join("state.sealed");
    let mut bytes = fs::read(&path).expect("read");
    bytes[0] ^= 1;
    fs::write(&path, bytes).expect("write");
    assert!(matches!(
        f.store.open(sealed.id, 1_500, &f.resumed),
        Err(StoreError::Seal(_))
    ));
    assert!(matches!(
        f.store.header(sealed.id),
        Err(StoreError::Missing)
    ));
}

#[test]
fn destroy_removes_members_and_key_and_may_be_repeated() {
    let f = fixture();
    stage(&f.staged, true);
    let sealed = f.store.seal(&f.staged, header(5)).expect("seal");
    f.store.destroy(sealed.id).expect("destroy");
    f.store.destroy(sealed.id).expect("destroy again");
    assert!(!f.root.join(sealed.id.hex()).exists());
    assert!(matches!(
        f.store.open(sealed.id, 1_500, &f.resumed),
        Err(StoreError::Missing)
    ));
}
