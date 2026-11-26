//! The broker's records of a published checkpoint's sealed memory and overlay, kept beside them.
//!
//! Every restore and fork otherwise re-reads the whole memory and the private head cloned from
//! the saved overlay. Capture already wrote each of them, synced it, and read it back against the
//! manifest's digest, so once the checkpoint is published the broker only records the stamp each
//! sealed file has, with that digest. A restore hands the memory record to the worker as it is.
//! For the overlay it clones the private head, checks the saved overlay still has its recorded
//! stamp, and vouches for the new head at the head's own stamp, all before the head's descriptor
//! leaves the broker. The worker skips a read only when the descriptor it maps still has exactly
//! the vouched stamp and the manifest names the vouched digest.
//!
//! The digest is the capture worker's, as every manifest digest already is. A worker that lied
//! about it could equally have written matching bytes and a matching manifest, so the restore
//! hash never defended against it; what the restore hash defends against is a change after
//! capture, and the stamp detects that without a read. A record is trusted exactly as far as the
//! private checkpoint directory is: whoever can write it can already write the file it describes.
//! A record that is absent, damaged, or stale only costs a hash.

use std::{
    fs::{self, File, OpenOptions},
    io::Write as _,
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
};

use soma_kvm::{
    snapshot::{ArtifactAttestation, Digest, FileStamp},
    x86_64::{Artifact, attest_all},
};

use super::{MEMORY, OVERLAY, open_private, read_bounded, sync_directory};

/// Each sealed artifact with a record, the record's name, and where the record is staged.
const RECORDS: [(&str, &str, &str); 2] = [
    (MEMORY, "memory.attest", "memory.attest.tmp"),
    (OVERLAY, "overlay.attest", "overlay.attest.tmp"),
];

/// Every name a record may have in a published checkpoint directory, staged or not.
pub(super) fn names() -> impl Iterator<Item = &'static str> {
    RECORDS
        .into_iter()
        .flat_map(|(_, record, staging)| [record, staging])
}

/// Whether `name` is a record, published or left staged by a crash, rather than part of the
/// checkpoint itself.
pub(super) fn is_record(name: &str) -> bool {
    names().any(|known| known == name)
}

/// Whether a file's stamps are trustworthy evidence that its bytes did not change.
///
/// Only filesystems known to move the change time on every write, including a write through a
/// shared mapping, qualify: XFS and ext4. On any other an attestation is never produced, so the
/// file is hashed.
pub(in crate::backend::kvm) fn stamps_hold(file: &File) -> bool {
    const XFS: rustix::fs::FsWord = 0x5846_5342;
    const EXT4: rustix::fs::FsWord = 0xEF53;
    rustix::fs::fstatfs(file).is_ok_and(|stat| matches!(stat.f_type, XFS | EXT4))
}

fn record_of(artifact: &str) -> Option<(&'static str, &'static str)> {
    RECORDS
        .into_iter()
        .find(|(name, _, _)| *name == artifact)
        .map(|(_, record, staging)| (record, staging))
}

/// Records the stamps of a just-published checkpoint's sealed memory and, when there is one,
/// saved overlay, with the digests capture sealed them against.
///
/// This runs after the directory rename, so each stamp is the one restore will see, it never
/// reads either file, and both wait out one settle together. Any failure leaves that record out,
/// and a restore then hashes the artifact.
pub(super) fn record(published: &Path, memory: Digest, overlay: Option<Digest>) {
    let open = |artifact: &str| {
        open_private(&published.join(artifact))
            .ok()
            .filter(stamps_hold)
    };
    let Some(memory_file) = open(MEMORY) else {
        return;
    };
    let overlay = overlay.and_then(|digest| Some((open(OVERLAY)?, digest)));
    let records = match &overlay {
        Some((file, digest)) => attest_all([
            (Artifact::Memory, &memory_file, memory),
            (Artifact::Overlay, file, *digest),
        ]),
        None => {
            attest_all([(Artifact::Memory, &memory_file, memory)]).map(|[memory]| [memory, None])
        }
    };
    let Ok(records) = records else {
        return;
    };
    // The records are in the order of `RECORDS`: memory, then overlay.
    for ((_, name, staging), attestation) in RECORDS.into_iter().zip(records) {
        if let Some(attestation) = attestation {
            let _ignored = write(published, (name, staging), &attestation);
        }
    }
}

/// The record of one sealed artifact of a published checkpoint, or `None` without a readable one.
pub(super) fn read(published: &Path, artifact: &str) -> Option<ArtifactAttestation> {
    let (name, _) = record_of(artifact)?;
    let bytes = read_bounded(&published.join(name)).ok()?;
    ArtifactAttestation::decode(std::str::from_utf8(&bytes).ok()?)
}

/// The broker's attestation of a private head it has just cloned from `saved`.
///
/// The clone carries the saved overlay's bytes only if the saved overlay was unchanged while it
/// was cloned, so its stamp must still equal its record after the clone. The head is then vouched
/// for at its own stamp, taken here before its descriptor leaves the broker; any later write to it
/// moves that stamp and the worker hashes it. Without a record for the saved overlay there is no
/// attestation, and the head is hashed as before.
pub(in crate::backend::kvm) fn vouch_head(
    saved: &File,
    record: Option<ArtifactAttestation>,
    head: &File,
) -> Option<ArtifactAttestation> {
    let record = record?;
    if !stamps_hold(saved) || !stamps_hold(head) {
        return None;
    }
    let saved = FileStamp::of(&saved.metadata().ok()?);
    let head = head.metadata().ok()?;
    (saved == record.stamp() && head.len() == record.stamp().size())
        .then(|| ArtifactAttestation::new(record.digest(), FileStamp::of(&head)))
}

fn write(
    published: &Path,
    (name, staging): (&str, &str),
    attestation: &ArtifactAttestation,
) -> std::io::Result<()> {
    let staging = published.join(staging);
    let _absent = fs::remove_file(&staging);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&staging)?;
    file.write_all(format!("{}\n", attestation.encode()).as_bytes())?;
    file.sync_all()?;
    fs::rename(&staging, published.join(name))?;
    sync_directory(published).map_err(|_| std::io::Error::other("sync checkpoint directory"))
}

#[cfg(test)]
mod tests;
