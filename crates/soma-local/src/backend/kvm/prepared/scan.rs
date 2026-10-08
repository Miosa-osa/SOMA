//! Finding the Generation a request asked for, and preloading every prepared entry.
//!
//! A host is expected to hold few prepared Generations, so this is a scan rather than an index:
//! an index would be a second source of truth about which bytes are prepared and could disagree
//! with the entries themselves. Every entry is examined, not just until one matches, because two
//! entries claiming one reference must fail as ambiguous rather than resolve by directory order.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use super::entry::{Claim, claims, read_bounded, read_entry};
use super::{
    GenerationCache, MAX_REFERENCE_BYTES, PreparedError, PreparedGeneration, REFERENCE, STORE,
};

/// Finds the Generation prepared for `reference` under `root`.
///
/// Every entry is examined, not just until one matches, because two entries claiming one
/// reference must fail as ambiguous rather than resolve by directory order. Which bytes a
/// request launches cannot depend on the order a filesystem happens to return names in, and
/// that decision is made before any entry's contents are read.
///
/// A host is expected to hold few prepared Generations, so this is a scan rather than an index:
/// an index would be a second source of truth about which bytes are prepared and could disagree
/// with the entries themselves.
pub(in crate::backend::kvm) fn find(
    root: Option<&Path>,
    reference: &str,
) -> Result<PreparedGeneration, PreparedError> {
    let root = root.ok_or(PreparedError::StoreUnset)?;
    let key = (root.to_path_buf(), reference.to_owned());
    if let Some(found) = cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        return Ok(found);
    }
    // A root that is simply absent is a different operator problem from one that is a link, and
    // reporting the wrong one sends the operator to the wrong place. Anything else that cannot
    // be described still counts as a link, because it cannot be shown not to be one.
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => return Err(PreparedError::Linked),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PreparedError::StoreUnreadable);
        }
        Err(_) => return Err(PreparedError::Linked),
    }
    let entries = std::fs::read_dir(root).map_err(|_| PreparedError::StoreUnreadable)?;
    let mut claimants = Vec::new();
    for entry in entries {
        // An entry that cannot be read is not skipped: an unreadable name could be the second
        // claimant that makes this reference ambiguous, so the scan fails rather than guessing.
        let path = entry.map_err(|_| PreparedError::StoreUnreadable)?.path();
        if !path.is_dir() {
            continue;
        }
        match claims(&path, reference) {
            Claim::Yes => claimants.push(path),
            Claim::No => {}
            // A claim that cannot be decided is not silently dropped, because the entry that
            // could not be read may be the one that makes this reference ambiguous.
            Claim::Unreadable => return Err(PreparedError::Damaged),
        }
    }
    let found = match claimants.as_slice() {
        [] => Err(PreparedError::NotPrepared),
        [only] => read_entry(root, only, reference),
        _ => Err(PreparedError::Ambiguous),
    }?;
    cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, found.clone());
    Ok(found)
}

/// Verifies and retains every named Generation before a hosted service accepts traffic.
pub(in crate::backend::kvm) fn preload() -> Result<(), PreparedError> {
    let root = store_root().ok_or(PreparedError::StoreUnset)?;
    let entries = std::fs::read_dir(&root).map_err(|_| PreparedError::StoreUnreadable)?;
    for entry in entries {
        let path = entry.map_err(|_| PreparedError::StoreUnreadable)?.path();
        if !path.is_dir() {
            continue;
        }
        let reference_path = path.join(REFERENCE);
        if !reference_path.exists() {
            continue;
        }
        let bytes =
            read_bounded(&reference_path, MAX_REFERENCE_BYTES).ok_or(PreparedError::Damaged)?;
        let reference = std::str::from_utf8(&bytes)
            .map_err(|_| PreparedError::Damaged)?
            .trim();
        if reference.is_empty() {
            return Err(PreparedError::Damaged);
        }
        find(Some(&root), reference)?;
    }
    Ok(())
}

fn cache() -> &'static GenerationCache {
    static CACHE: OnceLock<GenerationCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The prepared root this host names, if any.
pub(in crate::backend::kvm) fn store_root() -> Option<PathBuf> {
    std::env::var_os(STORE).map(PathBuf::from)
}
