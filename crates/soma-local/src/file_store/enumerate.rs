//! Reading the set of Instance identities this store holds records for.
//!
//! The store is a directory whose entries are named by Instance identity, so enumeration is a
//! directory read. It takes no lock. A per-Instance lock would serialise the enumeration against
//! every writer without making the answer a snapshot: an Instance created after this read began
//! is still not in it, so the honest description is "the identities that existed while this ran",
//! and the caller reads each record back under its own lock before reporting anything about it.
//!
//! A name that is not an Instance identity and is not the store's own lock directory is a
//! corruption rather than something to skip. Skipping it would make a store somebody wrote a
//! stray file into report a smaller set of sandboxes than it holds, silently.

use std::{fs, path::Path};

use soma::{InstanceId, StateStoreFailure};

use super::{
    failure::{corrupt, unavailable},
    layout::LOCK_DIRECTORY,
};

pub(super) fn instance_identities(root: &Path) -> Result<Vec<InstanceId>, StateStoreFailure> {
    let mut identities = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(corrupt());
        };
        if name == LOCK_DIRECTORY {
            continue;
        }
        let file_type = entry.file_type().map_err(|_| unavailable())?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(corrupt());
        }
        identities.push(InstanceId::new(name.to_owned()).map_err(|_| corrupt())?);
    }
    Ok(identities)
}
