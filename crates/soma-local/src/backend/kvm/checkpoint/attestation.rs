//! The broker's one hash of a published checkpoint's memory, kept beside it.
//!
//! Every restore and fork otherwise re-reads the whole memory file. Once the checkpoint is
//! published, the broker hashes `memory.raw` and records the stamp it had while it was hashed;
//! a restore hands that record to the worker, which skips the read only when the descriptor it
//! maps still has exactly that stamp and the manifest names that digest. The record is trusted
//! exactly as far as the private checkpoint directory is: whoever can write the record can
//! already write the memory file. A record that is absent, damaged, or stale only costs a hash.

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
};

use soma_kvm::{snapshot::MemoryAttestation, x86_64::attest_memory};

use super::{MEMORY, MEMORY_ATTESTATION, open_private, read_bounded, sync_directory};

/// Where a record is written before it replaces the published one.
pub(super) const STAGING: &str = "memory.attest.tmp";

/// Hashes the memory of a just-published checkpoint and records its stamp.
///
/// This runs after the directory rename, so the stamp is the one restore will see. Any failure
/// leaves no record, and a restore then hashes the memory as it would without one.
pub(super) fn record(published: &Path) {
    let Ok(mut memory) = open_private(&published.join(MEMORY)) else {
        return;
    };
    if let Ok(Some(attestation)) = attest_memory(&mut memory) {
        let _ignored = write(published, &attestation);
    }
}

/// The record of a published checkpoint, or `None` when there is no readable one.
pub(super) fn read(published: &Path) -> Option<MemoryAttestation> {
    let bytes = read_bounded(&published.join(MEMORY_ATTESTATION)).ok()?;
    MemoryAttestation::decode(std::str::from_utf8(&bytes).ok()?)
}

fn write(published: &Path, attestation: &MemoryAttestation) -> std::io::Result<()> {
    let staging = published.join(STAGING);
    let _absent = fs::remove_file(&staging);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&staging)?;
    file.write_all(format!("{}\n", attestation.encode()).as_bytes())?;
    file.sync_all()?;
    fs::rename(&staging, published.join(MEMORY_ATTESTATION))?;
    sync_directory(published).map_err(|_| std::io::Error::other("sync checkpoint directory"))
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write as _, os::unix::fs::OpenOptionsExt as _};

    use soma_kvm::snapshot::{Digest, FileStamp};

    use super::{MEMORY, MEMORY_ATTESTATION, read, record};

    fn published() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("checkpoint directory");
        let mut memory = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(directory.path().join(MEMORY))
            .expect("memory");
        memory.write_all(&[3_u8; 8192]).expect("write memory");
        directory
    }

    #[test]
    fn a_published_memory_file_is_recorded_at_its_current_stamp() {
        let directory = published();
        record(directory.path());
        let attestation = read(directory.path()).expect("recorded attestation");
        let memory = fs::File::open(directory.path().join(MEMORY)).expect("memory");
        assert_eq!(attestation.digest(), Digest::of(&[3_u8; 8192]));
        assert_eq!(
            attestation.stamp(),
            FileStamp::of(&memory.metadata().expect("stat"))
        );
    }

    #[test]
    fn a_missing_damaged_or_shared_record_is_no_attestation() {
        let directory = published();
        assert_eq!(read(directory.path()), None);
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(directory.path().join(MEMORY_ATTESTATION))
            .and_then(|mut file| file.write_all(b"not a record"))
            .expect("damage");
        assert_eq!(read(directory.path()), None);
        record(directory.path());
        assert!(read(directory.path()).is_some());
        let path = directory.path().join(MEMORY_ATTESTATION);
        let mut permissions = fs::metadata(&path).expect("stat").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o644);
        fs::set_permissions(&path, permissions).expect("chmod");
        assert_eq!(read(directory.path()), None);
    }
}
