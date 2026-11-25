//! Reading the set of Instance identities this store holds records for.
//!
//! The store is a directory whose entries are named by Instance identity, so enumeration is a
//! directory read. It takes no lock. A per-Instance lock would serialise the enumeration against
//! every writer without making the answer a snapshot: an Instance created after this read began
//! is still not in it, so the honest description is "the identities that existed while this ran",
//! and the caller reads each record back under its own lock before reporting anything about it.
//!
//! The root's known entries (the store's locks, the hosted machines' sockets, the sealed
//! Suspensions, the HTTP service's own directory, and the persisted prewarm set with its temporary
//! file) are not records. Any other
//! name that is not an Instance record directory is skipped rather than failing the whole read:
//! one stray file once made every listing on its host fail (SOMA#228). It is not skipped
//! silently; the first time this process meets it, it is named on the service's log.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use soma::{InstanceId, StateStoreFailure};

use super::{
    failure::unavailable,
    layout::{LOCK_DIRECTORY, MACHINE_HOST_DIRECTORY, SERVICE_DIRECTORY, SUSPENSION_DIRECTORY},
};
use crate::prewarm::PERSISTED_FILE;

pub(super) fn instance_identities(root: &Path) -> Result<Vec<InstanceId>, StateStoreFailure> {
    let mut identities = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            report_stray(&entry.path());
            continue;
        };
        if [
            LOCK_DIRECTORY,
            MACHINE_HOST_DIRECTORY,
            SUSPENSION_DIRECTORY,
            SERVICE_DIRECTORY,
        ]
        .contains(&name)
        {
            continue;
        }
        let file_type = entry.file_type().map_err(|_| unavailable())?;
        // The desired prewarm set lives beside the records (ADR 0052). It is a plain file, written
        // through a temporary name, and is not a record.
        if (name == PERSISTED_FILE || name.strip_suffix(".tmp") == Some(PERSISTED_FILE))
            && file_type.is_file()
        {
            continue;
        }
        match InstanceId::new(name.to_owned()) {
            Ok(identity) if file_type.is_dir() && !file_type.is_symlink() => {
                identities.push(identity);
            }
            _ => report_stray(&entry.path()),
        }
    }
    Ok(identities)
}

/// Names a stray entry on the service's log, once per process, so a listing that skips it every
/// minute does not repeat it every minute.
fn report_stray(path: &Path) {
    static REPORTED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
    let first = REPORTED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(path.to_path_buf());
    if first {
        eprintln!(
            "soma: {} is not a sandbox record; listings skip it",
            path.display()
        );
    }
}
