//! Proving a resident checkpoint's memory file still holds the bytes its manifest names.

use std::fs::File;

use super::SnapshotError;
use crate::{
    snapshot::{FileStamp, MemoryAttestation, memory::MemoryDescriptor},
    x86_64::snapshot::{Artifact, artifacts::hash},
};

/// Checks the memory file against its manifest record, hashing it unless the broker's
/// attestation vouches for this exact descriptor.
///
/// An attestation that names another stamp, size, or digest is ignored rather than refused: the
/// file is then hashed exactly as without one, so an attestation can only skip a read, never
/// admit bytes the manifest does not name.
///
/// # Errors
///
/// Returns the stat or read failure, a non-regular file, or the size or digest mismatch.
pub(super) fn verify(
    file: &mut File,
    record: &MemoryDescriptor,
    attestation: Option<MemoryAttestation>,
) -> Result<(), SnapshotError> {
    let metadata = file
        .metadata()
        .map_err(|error| SnapshotError::io(Artifact::Memory, "stat", &error))?;
    if !metadata.is_file() {
        return Err(SnapshotError::NotQuiescent(
            "checkpoint restore requires regular files",
        ));
    }
    let size = metadata.len();
    let vouched = attestation.and_then(|attestation| {
        attestation.vouches(FileStamp::of(&metadata), size, record.digest())
    });
    let digest = match vouched {
        Some(digest) => digest,
        None => hash(Artifact::Memory, file)?,
    };
    record.verify_generation(digest, size)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::{Seek as _, SeekFrom, Write as _},
        time::{Duration, Instant},
    };

    use super::verify;
    use crate::{
        snapshot::{Digest, FileStamp, MemoryAttestation, memory::MemoryDescriptor},
        x86_64::{SnapshotError, attest_memory},
    };

    const PAGE: usize = 4096;

    fn memory(bytes: &[u8]) -> File {
        let mut file = tempfile::tempfile().expect("memory file");
        file.write_all(bytes).expect("write memory");
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

    fn mismatch(result: Result<(), SnapshotError>) -> bool {
        matches!(
            result,
            Err(SnapshotError::Memory(
                crate::snapshot::memory::MemoryError::DigestMismatch { .. }
                    | crate::snapshot::memory::MemoryError::SizeMismatch { .. }
            ))
        )
    }

    #[test]
    fn an_attested_unchanged_file_is_not_read() {
        let captured = vec![7_u8; 2 * PAGE];
        // The file holds other bytes than the manifest names. Only a skipped read can admit it,
        // which is what proves the attestation replaced the hash.
        let mut file = memory(&vec![9_u8; 2 * PAGE]);
        let attestation = MemoryAttestation::new(Digest::of(&captured), stamp(&file));
        verify(&mut file, &record(&captured), Some(attestation)).expect("attested memory");
        assert!(mismatch(verify(&mut file, &record(&captured), None)));
    }

    #[test]
    fn a_file_changed_after_its_attestation_is_hashed_and_refused() {
        let captured = vec![7_u8; 2 * PAGE];
        let mut file = memory(&captured);
        let attestation = attest_memory(&mut file)
            .expect("hash")
            .expect("settled attestation");
        assert_eq!(attestation.digest(), Digest::of(&captured));
        file.seek(SeekFrom::Start(0)).expect("rewind");
        file.write_all(&[8]).expect("tamper");
        assert_ne!(stamp(&file), attestation.stamp());
        assert!(mismatch(verify(
            &mut file,
            &record(&captured),
            Some(attestation)
        )));
    }

    #[test]
    fn a_missing_attestation_is_hashed() {
        let captured = vec![7_u8; 2 * PAGE];
        let mut file = memory(&captured);
        verify(&mut file, &record(&captured), None).expect("hashed memory");
        let mut other = memory(&vec![1_u8; 2 * PAGE]);
        assert!(mismatch(verify(&mut other, &record(&captured), None)));
    }

    #[test]
    fn a_size_mismatch_is_hashed_and_refused() {
        let captured = vec![7_u8; 2 * PAGE];
        let mut file = memory(&vec![7_u8; 3 * PAGE]);
        let attestation = MemoryAttestation::new(Digest::of(&captured), stamp(&file));
        assert!(mismatch(verify(
            &mut file,
            &record(&captured),
            Some(attestation)
        )));
    }

    #[test]
    fn an_attestation_for_another_digest_is_hashed() {
        let captured = vec![7_u8; 2 * PAGE];
        let mut file = memory(&captured);
        let attestation = MemoryAttestation::new(Digest::of(b"another"), stamp(&file));
        verify(&mut file, &record(&captured), Some(attestation)).expect("hashed memory");
    }

    /// Times the check of a 1 GiB memory file with and without an attestation.
    ///
    /// Run with `cargo test -p soma-kvm --release resident_memory -- --ignored --nocapture`.
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
        file.sync_all().expect("sync");
        let began = Instant::now();
        let attestation = attest_memory(&mut file).expect("hash").expect("attest");
        let recorded = began.elapsed();
        let record = MemoryDescriptor::new(
            attestation.digest(),
            u64::try_from(GIB).expect("size"),
            4096,
        )
        .expect("record");
        let mut samples = |attested: Option<MemoryAttestation>| -> Vec<Duration> {
            (0..5)
                .map(|_| {
                    let began = Instant::now();
                    verify(&mut file, &record, attested).expect("verified");
                    began.elapsed()
                })
                .collect()
        };
        let hashed = samples(None);
        let skipped = samples(Some(attestation));
        eprintln!(
            "[resident-memory] 1 GiB: broker attest {recorded:?}; verify hashed {hashed:?}; verify attested {skipped:?}"
        );
    }
}
