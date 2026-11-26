//! Finding the Generation a request asked for, and preloading every prepared entry.
//!
//! A host is expected to hold few prepared Generations, so this is a scan rather than an index:
//! an index would be a second source of truth about which bytes are prepared and could disagree
//! with the entries themselves. Every entry is examined, not just until one matches, because two
//! entries claiming one reference must fail as ambiguous rather than resolve by directory order.

use std::path::{Path, PathBuf};

use super::cache::generations;
use super::entry::{Claim, claims, read_bounded, read_entry};
use super::{INCOMING, MAX_REFERENCE_BYTES, PreparedError, PreparedGeneration, REFERENCE, STORE};

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
    if let Some(found) = generations().get(&key) {
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
        if path.file_name().is_some_and(|name| name == INCOMING) || !path.is_dir() {
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
    let only = match claimants.as_slice() {
        [] => return Err(PreparedError::NotPrepared),
        [only] => only,
        _ => return Err(PreparedError::Ambiguous),
    };
    let found = read_entry(root, only, reference)?;
    // A prune that released this entry while it was being read has moved it out of the root, and
    // caching what was read would reopen what the prune just released.
    if !generations().admit(key, found.clone(), only) {
        return Err(PreparedError::NotPrepared);
    }
    Ok(found)
}

/// Verifies and retains every named Generation before a hosted service accepts traffic.
pub(in crate::backend::kvm) fn preload() -> Result<(), PreparedError> {
    let root = store_root().ok_or(PreparedError::StoreUnset)?;
    let entries = std::fs::read_dir(&root).map_err(|_| PreparedError::StoreUnreadable)?;
    for entry in entries {
        let path = entry.map_err(|_| PreparedError::StoreUnreadable)?.path();
        if path.file_name().is_some_and(|name| name == INCOMING) || !path.is_dir() {
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

/// Every entry in the root, each resolved through [`find`] as a launch would resolve it.
///
/// An entry that does not resolve is listed with the reason, so a damaged or uncertified entry
/// is visible rather than missing, and one bad entry never hides the rest.
pub(crate) fn catalog() -> Result<Vec<crate::PreparedSummary>, crate::CatalogError> {
    catalog_in(&store_root().ok_or(crate::CatalogError::Unsupported)?)
}

pub(super) fn catalog_in(root: &Path) -> Result<Vec<crate::PreparedSummary>, crate::CatalogError> {
    use crate::{CatalogError, PreparedSummary};
    const MIB: u64 = 1024 * 1024;
    let entries = std::fs::read_dir(root).map_err(|_| CatalogError::StoreUnavailable)?;
    let mut catalog = Vec::new();
    for entry in entries {
        let path = entry.map_err(|_| CatalogError::StoreUnavailable)?.path();
        if path.file_name().is_some_and(|name| name == INCOMING) || !path.is_dir() {
            continue;
        }
        let Some(bytes) = read_bounded(&path.join(REFERENCE), MAX_REFERENCE_BYTES) else {
            continue;
        };
        let Ok(reference) = std::str::from_utf8(&bytes).map(|text| text.trim().to_owned()) else {
            continue;
        };
        if reference.is_empty() {
            continue;
        }
        catalog.push(match find(Some(root), &reference) {
            Ok(found) => {
                let manifest = &found.manifest;
                let shape = &manifest.shape;
                let persistence = &manifest.persistence;
                PreparedSummary {
                    generation_id: Some(found.id.as_str().to_owned()),
                    vcpus: Some(shape.vcpu_count),
                    memory_mib: Some(shape.memory_bytes / MIB),
                    ceiling_mib: Some(shape.memory_bytes.saturating_add(shape.hotplug_bytes) / MIB),
                    disk_mib: Some(manifest.template.writable_storage_bytes / MIB),
                    volume_class_gib: persistence.enabled.then_some(persistence.size_class_gib),
                    compiler_policy: Some(manifest.compiler_policy_version),
                    refusal: None,
                    reference,
                }
            }
            Err(error) => PreparedSummary::refused(reference, refusal_code(&error)),
        });
    }
    catalog.sort_by(|a, b| a.reference.cmp(&b.reference));
    catalog.dedup_by(|a, b| a.reference == b.reference);
    Ok(catalog)
}

const fn refusal_code(error: &PreparedError) -> &'static str {
    match error {
        PreparedError::StoreUnset | PreparedError::StoreUnreadable => "store_unavailable",
        PreparedError::NotPrepared => "not_prepared",
        PreparedError::Ambiguous => "ambiguous",
        PreparedError::Uncertified => "uncertified",
        PreparedError::Linked => "linked",
        PreparedError::Damaged => "damaged",
    }
}

/// The prepared root this host names, if any.
pub(in crate::backend::kvm) fn store_root() -> Option<PathBuf> {
    std::env::var_os(STORE).map(PathBuf::from)
}
