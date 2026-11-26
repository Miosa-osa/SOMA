//! A trusted broker's one hash of a published checkpoint memory file.
//!
//! Restore and fork re-read the whole memory file to prove it still has the bytes the manifest
//! names, which is seconds per restore for a desktop. The broker that owns the checkpoint
//! directory hashes it once instead and records the stamp the file had while it was hashed, so a
//! restore of the unchanged file can skip the read. The order below is what makes the record
//! honest against any later write, including one through a descriptor or shared mapping a
//! writer kept:
//!
//! 1. The stamp is taken before hashing and must be the same after it.
//! 2. Hashing starts only once the clock has moved well past the stamp's change time, so a later
//!    write cannot land on the same change time as the stamped content.
//! 3. The file is synced after the stamp, which cleans and write-protects every page of a shared
//!    mapping, so a later write through one faults and moves the change time as a write call does.

use std::{
    fs::File,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{Artifact, SnapshotError, artifacts::hash};
use crate::snapshot::{FileStamp, MemoryAttestation};

/// How far past the stamped change time the clock must be before hashing starts.
///
/// File times come from the kernel's coarse clock, which lags real time by up to one tick.
const SETTLE: Duration = Duration::from_millis(50);

/// Hashes a broker-owned memory file and returns the attestation of its current bytes.
///
/// Returns `None` when the file changed while it was hashed or when its change time is not
/// safely in the past, in which case a restore simply hashes it again.
///
/// # Errors
///
/// Returns the stat, sync, or read failure.
pub fn attest_memory(file: &mut File) -> Result<Option<MemoryAttestation>, SnapshotError> {
    let before = stamp(file)?;
    let Some(changed) = instant(before.changed()) else {
        return Ok(None);
    };
    let ready = changed + SETTLE;
    let now = SystemTime::now();
    match ready.duration_since(now) {
        Ok(wait) if wait > SETTLE => return Ok(None),
        Ok(wait) => std::thread::sleep(wait),
        Err(_) => {}
    }
    file.sync_all()
        .map_err(|error| SnapshotError::io(Artifact::Memory, "sync", &error))?;
    let digest = hash(Artifact::Memory, file)?;
    Ok((stamp(file)? == before).then_some(MemoryAttestation::new(digest, before)))
}

fn stamp(file: &File) -> Result<FileStamp, SnapshotError> {
    file.metadata()
        .map(|metadata| FileStamp::of(&metadata))
        .map_err(|error| SnapshotError::io(Artifact::Memory, "stat", &error))
}

fn instant((seconds, nanoseconds): (i64, i64)) -> Option<SystemTime> {
    let offset = Duration::new(
        u64::try_from(seconds).ok()?,
        u32::try_from(nanoseconds).ok()?,
    );
    UNIX_EPOCH.checked_add(offset)
}
