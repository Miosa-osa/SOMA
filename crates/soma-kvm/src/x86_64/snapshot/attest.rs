//! A trusted broker's record of a sealed artifact, taken without reading it again.
//!
//! Restore and fork re-read whole artifacts to prove they still hold the bytes their records
//! name, which is seconds per GiB. An artifact that was already hashed when it was sealed (a
//! checkpoint's memory, which capture writes, syncs, and reads back against its manifest digest)
//! does not need a second read: the broker only records the stamp the sealed file has now, so a
//! restore of the unchanged file can skip the read. The order below is what makes that record
//! honest against any later write, including one through a descriptor or shared mapping a
//! writer kept:
//!
//! 1. The file is synced first, which cleans and write-protects every page of a shared mapping,
//!    so a later write through one faults and moves the change time as a write call does.
//! 2. The stamp is taken, and the record is made only once the clock has moved well past its
//!    change time, so a later write cannot land on the same change time on a coarse clock.
//! 3. The stamp is taken again and must be unchanged.
//!
//! The digest is the one the sealing step verified, not one this module computed. The record is
//! therefore exactly as trustworthy as that sealing step for the bytes it wrote, and it proves
//! only that nothing changed them afterwards.

use std::{
    fs::File,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{Artifact, SnapshotError};
use crate::snapshot::{ArtifactAttestation, Digest, FileStamp};

/// How far past the stamped change time the clock must be before the stamp is recorded.
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
    file: &File,
    digest: Digest,
) -> Result<Option<ArtifactAttestation>, SnapshotError> {
    file.sync_all()
        .map_err(|error| SnapshotError::io(Artifact::Memory, "sync", &error))?;
    let before = stamp(file)?;
    let Some(changed) = instant(before.changed()) else {
        return Ok(None);
    };
    match (changed + SETTLE).duration_since(SystemTime::now()) {
        Ok(wait) if wait > SETTLE => return Ok(None),
        Ok(wait) => std::thread::sleep(wait),
        Err(_) => {}
    }
    Ok((stamp(file)? == before).then_some(ArtifactAttestation::new(digest, before)))
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
