use std::{
    fs::File,
    io::{Seek as _, SeekFrom, Write as _},
    time::{Duration, Instant},
};

use super::{digest, verify_memory};
use crate::{
    snapshot::{
        ArtifactAttestation, Digest, FileStamp,
        memory::{MemoryDescriptor, MemoryError},
    },
    x86_64::{Artifact, SnapshotError, attest_sealed},
};

const PAGE: usize = 4096;

fn file_with(bytes: &[u8]) -> File {
    let mut file = tempfile::tempfile().expect("scratch file");
    file.write_all(bytes).expect("write");
    file
}

fn record(bytes: &[u8]) -> MemoryDescriptor {
    MemoryDescriptor::new(
        Digest::of(bytes),
        u64::try_from(bytes.len()).expect("size"),
        4096,
    )
    .expect("record")
}

fn stamp(file: &File) -> FileStamp {
    FileStamp::of(&file.metadata().expect("stat"))
}

fn mismatch(result: &Result<(), SnapshotError>) -> bool {
    matches!(
        result,
        Err(SnapshotError::Memory(
            MemoryError::DigestMismatch { .. } | MemoryError::SizeMismatch { .. }
        ))
    )
}

#[test]
fn an_attested_unchanged_file_is_not_read() {
    let captured = vec![7_u8; 2 * PAGE];
    // The file holds other bytes than the manifest names. Only a skipped read can admit it,
    // which is what proves the attestation replaced the hash.
    let mut file = file_with(&vec![9_u8; 2 * PAGE]);
    let attestation = ArtifactAttestation::new(Digest::of(&captured), stamp(&file));
    verify_memory(&mut file, &record(&captured), Some(attestation)).expect("attested memory");
    assert!(mismatch(&verify_memory(
        &mut file,
        &record(&captured),
        None
    )));
}

#[test]
fn a_file_changed_after_its_attestation_is_hashed_and_refused() {
    let captured = vec![7_u8; 2 * PAGE];
    let mut file = file_with(&captured);
    let attestation = attest_sealed(Artifact::Memory, &file, Digest::of(&captured))
        .expect("stamp")
        .expect("settled attestation");
    file.seek(SeekFrom::Start(0)).expect("rewind");
    file.write_all(&[8]).expect("tamper");
    assert_ne!(stamp(&file), attestation.stamp());
    assert!(mismatch(&verify_memory(
        &mut file,
        &record(&captured),
        Some(attestation)
    )));
}

#[test]
fn a_missing_attestation_is_hashed() {
    let captured = vec![7_u8; 2 * PAGE];
    let mut file = file_with(&captured);
    verify_memory(&mut file, &record(&captured), None).expect("hashed memory");
    let mut other = file_with(&vec![1_u8; 2 * PAGE]);
    assert!(mismatch(&verify_memory(
        &mut other,
        &record(&captured),
        None
    )));
}

#[test]
fn a_size_mismatch_is_hashed_and_refused() {
    let captured = vec![7_u8; 2 * PAGE];
    let mut file = file_with(&vec![7_u8; 3 * PAGE]);
    let attestation = ArtifactAttestation::new(Digest::of(&captured), stamp(&file));
    assert!(mismatch(&verify_memory(
        &mut file,
        &record(&captured),
        Some(attestation)
    )));
}

#[test]
fn an_attestation_for_another_digest_is_hashed() {
    let captured = vec![7_u8; 2 * PAGE];
    let mut file = file_with(&captured);
    let attestation = ArtifactAttestation::new(Digest::of(b"another"), stamp(&file));
    verify_memory(&mut file, &record(&captured), Some(attestation)).expect("hashed memory");
}

#[test]
fn an_attested_root_is_not_read_and_a_touched_root_is() {
    let certified = vec![5_u8; 2 * PAGE];
    let mut root = file_with(&vec![6_u8; 2 * PAGE]);
    let expected = Digest::of(&certified);
    let attestation = attest_sealed(Artifact::Root, &root, expected)
        .expect("stamp")
        .expect("settled attestation");
    let metadata = root.metadata().expect("stat");
    let read = digest(
        Artifact::Root,
        &mut root,
        &metadata,
        Some(attestation),
        expected,
    );
    assert_eq!(read.expect("vouched root"), expected);
    // An in-place write keeps the size, so only the change time can reveal it.
    std::thread::sleep(Duration::from_millis(20));
    root.seek(SeekFrom::Start(0)).expect("rewind");
    root.write_all(&[1]).expect("touch");
    let metadata = root.metadata().expect("stat");
    let read = digest(
        Artifact::Root,
        &mut root,
        &metadata,
        Some(attestation),
        expected,
    );
    assert_ne!(read.expect("hashed root"), expected);
}

#[test]
fn a_head_modified_after_the_broker_stamped_it_is_hashed_and_refused() {
    let captured = vec![4_u8; 2 * PAGE];
    let expected = Digest::of(&captured);
    // The broker's head step: a fresh clone of the saved overlay, stamped before handover.
    let mut head = file_with(&captured);
    let attestation = ArtifactAttestation::new(expected, stamp(&head));
    let metadata = head.metadata().expect("stat");
    let read = digest(
        Artifact::Overlay,
        &mut head,
        &metadata,
        Some(attestation),
        expected,
    );
    assert_eq!(read.expect("vouched head"), expected);
    // Any write after the stamp, even one restoring the size, moves the change time.
    std::thread::sleep(Duration::from_millis(20));
    head.seek(SeekFrom::Start(4096)).expect("seek");
    head.write_all(&[9]).expect("tamper with the head");
    let metadata = head.metadata().expect("stat");
    assert_ne!(FileStamp::of(&metadata), attestation.stamp());
    let read = digest(
        Artifact::Overlay,
        &mut head,
        &metadata,
        Some(attestation),
        expected,
    );
    assert_ne!(
        read.expect("hashed head"),
        expected,
        "a tampered head must be refused"
    );
}

#[test]
fn sealing_waits_out_a_fresh_change_time_and_never_reads() {
    let mut file = file_with(&[3_u8; PAGE]);
    let began = Instant::now();
    let attestation = attest_sealed(Artifact::Memory, &file, Digest::of(b"never computed here"))
        .expect("stamp")
        .expect("settled attestation");
    assert!(began.elapsed() < Duration::from_secs(1));
    // The digest is the caller's, which only a skipped read would ever return.
    let metadata = file.metadata().expect("stat");
    let vouched = digest(
        Artifact::Memory,
        &mut file,
        &metadata,
        Some(attestation),
        Digest::of(b"never computed here"),
    );
    assert_eq!(
        vouched.expect("vouched"),
        Digest::of(b"never computed here")
    );
}

/// Times the memory check of a 1 GiB file with and without an attestation, and the attestation.
///
/// Run with `cargo test -p soma-kvm --release attested -- --ignored --nocapture`.
#[test]
#[ignore = "writes and hashes one GiB; run by hand for the measurement"]
fn measure_one_gib_with_and_without_an_attestation() {
    const GIB: usize = 1 << 30;
    let chunk: Vec<u8> = (0..=u8::MAX).cycle().take(1 << 20).collect();
    let mut file = tempfile::tempfile_in(
        std::env::var_os("SOMA_MEASURE_DIR").unwrap_or_else(|| std::env::temp_dir().into()),
    )
    .expect("scratch");
    for _ in 0..GIB / chunk.len() {
        file.write_all(&chunk).expect("fill");
    }
    // Capture's seal has already synced the file by the time the broker records it.
    file.sync_all().expect("sync as seal does");
    let began = Instant::now();
    let digest =
        crate::x86_64::snapshot::artifacts::hash(Artifact::Memory, &mut file).expect("hash");
    let hashed_once = began.elapsed();
    let began = Instant::now();
    let attestation = attest_sealed(Artifact::Memory, &file, digest)
        .expect("stamp")
        .expect("attest");
    let sealed = began.elapsed();
    let record =
        MemoryDescriptor::new(digest, u64::try_from(GIB).expect("size"), 4096).expect("record");
    let mut samples = |attested: Option<ArtifactAttestation>| -> Vec<Duration> {
        (0..5)
            .map(|_| {
                let began = Instant::now();
                verify_memory(&mut file, &record, attested).expect("verified");
                began.elapsed()
            })
            .collect()
    };
    let hashed = samples(None);
    let skipped = samples(Some(attestation));
    eprintln!(
        "[attested] 1 GiB: one full hash {hashed_once:?}; broker attestation {sealed:?}; verify hashed {hashed:?}; verify attested {skipped:?}"
    );
}

/// Reports the size and full hash time of each named file, as restore would read it.
///
/// Run with `SOMA_MEASURE_FILES=a:b cargo test -p soma-kvm --release attested -- --ignored`.
#[test]
#[ignore = "reads whole named files; run by hand for the measurement"]
fn measure_named_files() {
    let Some(names) = std::env::var_os("SOMA_MEASURE_FILES") else {
        return;
    };
    for path in std::env::split_paths(&names) {
        let mut file = File::open(&path).expect("open");
        let size = file.metadata().expect("stat").len();
        let began = Instant::now();
        crate::x86_64::snapshot::artifacts::hash(Artifact::Root, &mut file).expect("hash");
        let elapsed = began.elapsed();
        let metadata = file.metadata().expect("stat");
        let held = ArtifactAttestation::new(Digest::of(b"measured"), FileStamp::of(&metadata));
        let began = Instant::now();
        let vouched = digest(
            Artifact::Root,
            &mut file,
            &metadata,
            Some(held),
            held.digest(),
        );
        let skipped = began.elapsed();
        assert_eq!(vouched.expect("vouched"), held.digest());
        eprintln!(
            "[attested] {size} bytes ({} MiB) hashed in {elapsed:?}, attested in {skipped:?}: {}",
            size >> 20,
            path.display()
        );
    }
}
