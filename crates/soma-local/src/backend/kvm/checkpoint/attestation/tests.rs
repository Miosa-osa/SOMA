use std::{
    fs::{self, File},
    io::Write as _,
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
};

use soma_kvm::snapshot::{Digest, FileStamp};

use super::{MEMORY, OVERLAY, is_record, names, read, record, stamps_hold, vouch_head};

fn private(path: &Path, bytes: &[u8]) -> File {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .expect("private file");
    file.write_all(bytes).expect("write");
    file
}

fn published() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("checkpoint directory");
    private(&directory.path().join(MEMORY), &[3_u8; 8192]);
    private(&directory.path().join(OVERLAY), &[5_u8; 8192]);
    directory
}

fn stamp(path: &Path) -> FileStamp {
    FileStamp::of(&fs::metadata(path).expect("stat"))
}

#[test]
fn a_published_artifact_is_recorded_at_its_current_stamp_without_a_read() {
    let directory = published();
    // The recorded digests are the sealed ones handed in, never ones computed from the files.
    let sealed = |artifact: &str| Digest::of(format!("sealed {artifact}").as_bytes());
    record(directory.path(), sealed(MEMORY), Some(sealed(OVERLAY)));
    for artifact in [MEMORY, OVERLAY] {
        let attestation = read(directory.path(), artifact).expect("recorded attestation");
        assert_eq!(attestation.digest(), sealed(artifact));
        assert_eq!(attestation.stamp(), stamp(&directory.path().join(artifact)));
    }
    assert_eq!(names().count(), 4);
    for staged in ["memory.attest.tmp", "overlay.attest.tmp", "overlay.attest"] {
        assert!(is_record(staged), "{staged}");
    }
    assert!(!is_record(OVERLAY) && !is_record("receipt.json"));
}

#[test]
fn a_missing_damaged_or_shared_record_is_no_attestation() {
    let directory = published();
    assert_eq!(read(directory.path(), MEMORY), None);
    private(&directory.path().join("memory.attest"), b"not a record");
    assert_eq!(read(directory.path(), MEMORY), None);
    fs::remove_file(directory.path().join("memory.attest")).expect("remove");
    record(directory.path(), Digest::of(&[3_u8; 8192]), None);
    assert!(read(directory.path(), MEMORY).is_some());
    assert_eq!(read(directory.path(), OVERLAY), None);
    let path = directory.path().join("memory.attest");
    let mut permissions = fs::metadata(&path).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o644);
    fs::set_permissions(&path, permissions).expect("chmod");
    assert_eq!(read(directory.path(), MEMORY), None);
}

#[test]
fn no_attestation_is_produced_on_a_filesystem_whose_stamps_do_not_hold() {
    let Ok(directory) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    private(&directory.path().join(MEMORY), &[3_u8; 8192]);
    let overlay = private(&directory.path().join(OVERLAY), &[5_u8; 8192]);
    assert!(!stamps_hold(&overlay), "tmpfs stamps must not be trusted");
    record(
        directory.path(),
        Digest::of(b"memory"),
        Some(Digest::of(b"overlay")),
    );
    assert_eq!(read(directory.path(), MEMORY), None);
    assert_eq!(read(directory.path(), OVERLAY), None);
    let head = private(&directory.path().join("head"), &[5_u8; 8192]);
    let recorded = soma_kvm::snapshot::ArtifactAttestation::new(
        Digest::of(b"overlay"),
        FileStamp::of(&overlay.metadata().expect("stat")),
    );
    assert_eq!(vouch_head(&overlay, Some(recorded), &head), None);
}

#[test]
fn a_fresh_head_of_an_unchanged_attested_overlay_is_vouched_at_its_own_stamp() {
    let directory = published();
    let saved_path = directory.path().join(OVERLAY);
    record(
        directory.path(),
        Digest::of(&[3_u8; 8192]),
        Some(Digest::of(&[5_u8; 8192])),
    );
    let saved = File::open(&saved_path).expect("saved overlay");
    let head_path = directory.path().join("head");
    fs::copy(&saved_path, &head_path).expect("clone");
    let head = File::open(&head_path).expect("head");
    let vouched = vouch_head(&saved, read(directory.path(), OVERLAY), &head).expect("vouched head");
    assert_eq!(vouched.digest(), Digest::of(&[5_u8; 8192]));
    assert_eq!(vouched.stamp(), stamp(&head_path));
    assert_ne!(vouched.stamp(), stamp(&saved_path));
}

#[test]
fn a_head_is_not_vouched_without_a_record_or_after_the_saved_overlay_changed() {
    let directory = published();
    let saved_path = directory.path().join(OVERLAY);
    let head_path = directory.path().join("head");
    fs::copy(&saved_path, &head_path).expect("clone");
    let head = File::open(&head_path).expect("head");
    let saved = File::open(&saved_path).expect("saved overlay");
    assert_eq!(vouch_head(&saved, None, &head), None);
    record(
        directory.path(),
        Digest::of(&[3_u8; 8192]),
        Some(Digest::of(&[5_u8; 8192])),
    );
    let recorded = read(directory.path(), OVERLAY);
    // An in-place write keeps the size, so only the change time can reveal it.
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::OpenOptions::new()
        .write(true)
        .open(&saved_path)
        .and_then(|mut file| file.write_all(&[0]))
        .expect("change the saved overlay");
    assert_eq!(vouch_head(&saved, recorded, &head), None);
}

/// Times the broker's restore step for a desktop-size saved overlay: the record made at
/// publication, the head clone, and the head vouch, none of which reads the overlay.
///
/// Run with `SOMA_HEAD_DIR=<reflink dir> SOMA_MEASURE_OVERLAY=<private copy> cargo test
/// -p soma-local --release measure_head_vouch -- --ignored --nocapture`.
#[test]
#[ignore = "clones a named desktop-size overlay; run by hand for the measurement"]
fn measure_head_vouch() {
    use std::time::Instant;
    let Some(path) = std::env::var_os("SOMA_MEASURE_OVERLAY") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    let directory = path.parent().expect("published directory");
    let memory = directory.join(MEMORY);
    if !memory.exists() {
        private(&memory, &[0_u8; 4096]);
    }
    let began = Instant::now();
    record(
        directory,
        Digest::of(b"memory"),
        Some(Digest::of(b"sealed by capture")),
    );
    let recorded = began.elapsed();
    let saved = File::open(&path).expect("saved overlay");
    let instance = soma::InstanceId::new("ab".repeat(16)).expect("instance");
    let began = Instant::now();
    let head = crate::backend::kvm::boot::private_head_from(saved.try_clone().unwrap(), &instance)
        .expect("clone head");
    let cloned = began.elapsed();
    let began = Instant::now();
    let vouched = vouch_head(&saved, read(directory, OVERLAY), &head).expect("vouched head");
    let vouching = began.elapsed();
    assert_eq!(
        vouched.stamp(),
        FileStamp::of(&head.metadata().expect("stat"))
    );
    eprintln!(
        "[head-vouch] {} MiB overlay: record at publication {recorded:?}; head clone {cloned:?}; head vouch {vouching:?}",
        saved.metadata().expect("stat").len() >> 20
    );
}
