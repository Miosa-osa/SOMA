//! A trusted broker's record of sealed artifacts, taken without reading them again.
//!
//! Restore and fork re-read whole artifacts to prove they still hold the bytes their records
//! name, which is seconds per GiB. An artifact that was already hashed when it was sealed (a
//! checkpoint's memory or saved overlay, which capture writes, syncs, and reads back against its
//! manifest digest) does not need a second read: the broker only records the stamp the sealed
//! file has now, so a restore of the unchanged file can skip the read.
//!
//! The order below makes the record detect any later write by a party other than the capture
//! worker:
//!
//! 1. Each file is synced first, which cleans and write-protects every page of a shared mapping,
//!    so a later write through one faults and moves the change time as a write call does.
//! 2. Each stamp is taken, and the records are made only once the clock has moved well past the
//!    newest change time, so a write made after that point cannot reuse a recorded change time
//!    on a coarse clock.
//! 3. Each stamp is taken again and must be unchanged.
//!
//! What this does not stop is the capture worker itself, if it kept a writable descriptor or
//! mapping, writing within the same clock tick as its own last write, before the broker's first
//! stamp. That gives it nothing it lacks: the digest recorded here is the worker's own sealed
//! digest, so a worker willing to deceive could already write any bytes together with a manifest
//! that names them. The record is exactly as trustworthy as that sealing step for the bytes it
//! wrote, and it proves only that nothing changed them afterwards.

use std::{
    fs::File,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{Artifact, SnapshotError};
use crate::snapshot::{ArtifactAttestation, Digest, FileStamp};

/// How far past the newest stamped change time the clock must be before the stamps are recorded.
///
/// File times come from the kernel's coarse clock, which lags real time by up to one tick.
const SETTLE: Duration = Duration::from_millis(50);

/// Records that the sealed `file` holds the bytes whose digest is `digest`, without reading it.
///
/// Returns `None` when the file changed while it was stamped or when its change time is not
/// safely in the past, in which case a restore simply hashes it.
///
/// # Errors
///
/// Returns the stat or sync failure.
pub fn attest_sealed(
    artifact: Artifact,
    file: &File,
    digest: Digest,
) -> Result<Option<ArtifactAttestation>, SnapshotError> {
    let [record] = attest_all([(artifact, file, digest)])?;
    Ok(record)
}

/// Records each sealed file as [`attest_sealed`] does, waiting out one settle for all of them.
///
/// # Errors
///
/// Returns the first stat or sync failure.
pub fn attest_all<const N: usize>(
    sealed: [(Artifact, &File, Digest); N],
) -> Result<[Option<ArtifactAttestation>; N], SnapshotError> {
    let mut before = [FileStamp::new(0, 0, 0, (0, 0), (0, 0)); N];
    for (slot, (artifact, file, _)) in before.iter_mut().zip(&sealed) {
        file.sync_all()
            .map_err(|error| SnapshotError::io(*artifact, "sync", &error))?;
        *slot = stamp(*artifact, file)?;
    }
    let mut newest = UNIX_EPOCH;
    for stamp in &before {
        let Some(changed) = instant(stamp.changed()) else {
            return Ok([None; N]);
        };
        newest = newest.max(changed);
    }
    match (newest + SETTLE).duration_since(SystemTime::now()) {
        Ok(wait) if wait > SETTLE => return Ok([None; N]),
        Ok(wait) => std::thread::sleep(wait),
        Err(_) => {}
    }
    let mut records = [None; N];
    for ((record, (artifact, file, digest)), before) in records.iter_mut().zip(sealed).zip(before) {
        *record =
            (stamp(artifact, file)? == before).then_some(ArtifactAttestation::new(digest, before));
    }
    Ok(records)
}

fn stamp(artifact: Artifact, file: &File) -> Result<FileStamp, SnapshotError> {
    file.metadata()
        .map(|metadata| FileStamp::of(&metadata))
        .map_err(|error| SnapshotError::io(artifact, "stat", &error))
}

fn instant((seconds, nanoseconds): (i64, i64)) -> Option<SystemTime> {
    let offset = Duration::new(
        u64::try_from(seconds).ok()?,
        u32::try_from(nanoseconds).ok()?,
    );
    UNIX_EPOCH.checked_add(offset)
}
