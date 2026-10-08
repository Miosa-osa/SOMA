//! Finding the one prepared Generation a request is addressed to.
//!
//! Claiming is decided by the reference text alone, before anything else is read, so that two
//! entries claiming one reference are ambiguous whatever their contents are. Which bytes a
//! request launches must never depend on the order a filesystem happens to return names in.

use std::path::Path;
use std::sync::Arc;

use soma::GenerationId;
use soma_generation::{CompilerProfile, admit_installed_generation};

use super::{
    CANDIDATE, GENERATION_ID, GENERATION_ID_BYTES, MAX_REFERENCE_BYTES, PreparedError,
    PreparedGeneration, REFERENCE, STORE_DIRECTORY, any_component_is_link, cache, is_link,
};

/// Whether one entry claims `reference`, or cannot be read well enough to say.
///
/// An entry whose reference file is absent is simply not addressed to any request. An entry whose
/// reference file exists but is oversized, unreadable, or not text is a different thing: it may be
/// the second claimant that makes this reference ambiguous, and treating it as a non-claim would
/// let a damaged entry disappear from that check instead of failing the scan closed.
enum Claim {
    /// The entry names this reference.
    Yes,
    /// The entry names something else, or names nothing at all.
    No,
    /// The entry cannot be read well enough to decide.
    Unreadable,
}

fn claims(entry: &Path, reference: &str) -> Claim {
    let path = entry.join(REFERENCE);
    if !path.exists() {
        return Claim::No;
    }
    let Some(bytes) = read_bounded(&path, MAX_REFERENCE_BYTES) else {
        return Claim::Unreadable;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Claim::Unreadable;
    };
    if text.trim() == reference {
        Claim::Yes
    } else {
        Claim::No
    }
}

/// Reads at most `limit` bytes, refusing anything larger.
///
/// These files come from a host directory the request path does not control, so a read is
/// bounded rather than trusted to be small. A file at the limit is refused too, because a file
/// that fills the bound may have been cut off at it.
pub(super) fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 >= limit {
        return None;
    }
    Some(bytes)
}

/// Reads the one entry that claims the reference, once it is known to be the only one.
fn read_entry(
    root: &Path,
    entry: &Path,
    reference: &str,
) -> Result<PreparedGeneration, PreparedError> {
    let generation_id = entry.join(GENERATION_ID);
    let store = entry.join(STORE_DIRECTORY);
    if any_component_is_link(root, entry) || is_link(&entry.join(REFERENCE)) || is_link(&store) {
        return Err(PreparedError::Linked);
    }
    if !generation_id.exists() {
        return if entry.join(CANDIDATE).is_file() {
            Err(PreparedError::Uncertified)
        } else {
            Err(PreparedError::Damaged)
        };
    }
    if is_link(&generation_id) {
        return Err(PreparedError::Linked);
    }
    if !store.is_dir() {
        return Err(PreparedError::Damaged);
    }
    let bytes = read_bounded(&generation_id, GENERATION_ID_BYTES).ok_or(PreparedError::Damaged)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| PreparedError::Damaged)?;
    let id = GenerationId::new(text.trim().to_owned()).map_err(|_| PreparedError::Damaged)?;
    let admitted = admit_installed_generation(&store, &id, &CompilerProfile::v1())
        .map_err(|_| PreparedError::Damaged)?;
    let admitted_id = admitted.id.clone();
    let (manifest, artifacts) = admitted.into_parts();
    let artifacts = artifacts
        .into_iter()
        .map(|(descriptor, file)| (descriptor, Arc::new(file)))
        .collect();
    Ok(PreparedGeneration {
        store,
        reference: reference.to_owned(),
        id: admitted_id,
        manifest,
        artifacts,
    })
}

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
