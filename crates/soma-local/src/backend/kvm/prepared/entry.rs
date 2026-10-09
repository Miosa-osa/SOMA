//! Reading one prepared entry, and rebuilding it from handles a launching parent transferred.
//!
//! Everything here reads from a host directory the request path does not control, so no path is
//! trusted: a symbolic link anywhere below the root is refused rather than followed, a bounded
//! read is refused at its own limit, and identity is recomputed from the bytes rather than
//! recorded beside them.

use std::{
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
};

use soma::GenerationId;
use soma_generation::{
    CompilerProfile, admit_installed_generation, admit_verified_handoff, declared_policy_version,
    installed_policy_version,
};

use super::{
    CANDIDATE, GENERATION_ID, GENERATION_ID_BYTES, MAX_REFERENCE_BYTES, PreparedError,
    PreparedGeneration, REFERENCE, STORE_DIRECTORY,
};

/// Rebuilds a prepared Generation from verified handles transferred by its launching parent.
pub(in crate::backend::kvm) fn from_handoff(
    reference: String,
    id: &GenerationId,
    manifest: &[u8],
    descriptors: Vec<OwnedFd>,
) -> Result<PreparedGeneration, PreparedError> {
    let files = descriptors.into_iter().map(std::fs::File::from).collect();
    // The profile is the one the manifest itself declares. A Generation built under a newer
    // machine contract cannot be admitted by a host that assumed the older one, and the manifest
    // a launching parent already verified is exactly what names the contract it holds.
    let profile =
        profile_for(declared_policy_version(manifest).map_err(|_| PreparedError::Damaged)?)?;
    let admitted = admit_verified_handoff(id, manifest, files, &profile)
        .map_err(|_| PreparedError::Damaged)?;
    let admitted_id = admitted.id.clone();
    let (manifest, artifacts) = admitted.into_parts();
    let artifacts = artifacts
        .into_iter()
        .map(|(descriptor, file)| (descriptor, Arc::new(file)))
        .collect();
    Ok(PreparedGeneration {
        store: PathBuf::new(),
        reference,
        id: admitted_id,
        manifest,
        artifacts,
    })
}

/// The compiler profile one declared compiler-policy version names.
///
/// A prepared entry whose policy has no profile in this build is refused as damaged rather than
/// admitted under a profile that describes a different machine.
fn profile_for(version: u16) -> Result<CompilerProfile, PreparedError> {
    CompilerProfile::from_policy_version(version).ok_or(PreparedError::Damaged)
}

/// Whether `path` is a symbolic link, treating an unreadable path as one.
///
/// `symlink_metadata` does not follow the final component, so this reports the link itself
/// rather than what it points at. A path that cannot be read at all is refused for the same
/// reason a link is: what launches must be exactly what was verified.
pub(super) fn is_link(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_symlink(),
        // A path that cannot be described cannot be shown not to be a link, so it counts as one.
        // `is_ok_and` would answer false here, which is the opposite of failing closed.
        Err(_) => true,
    }
}

/// Whether `path` or any component of it below `root` is a symbolic link.
///
/// Checking only the final component leaves an ancestor free to redirect everything beneath it,
/// so each component from `root` down is examined.
fn any_component_is_link(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let mut walked = root.to_path_buf();
    for component in relative.components() {
        walked.push(component);
        if is_link(&walked) {
            return true;
        }
    }
    false
}

/// Whether this entry claims `reference`.
///
/// Claiming is decided by the reference text alone, before anything else is read, so that two
/// entries claiming one reference are ambiguous whatever their contents are.
/// Whether one entry claims `reference`, or cannot be read well enough to say.
///
/// An entry whose reference file is absent is simply not addressed to any request. An entry whose
/// reference file exists but is oversized, unreadable, or not text is a different thing: it may be
/// the second claimant that makes this reference ambiguous, and treating it as a non-claim would
/// let a damaged entry disappear from that check instead of failing the scan closed.
pub(super) enum Claim {
    /// The entry names this reference.
    Yes,
    /// The entry names something else, or names nothing at all.
    No,
    /// The entry cannot be read well enough to decide.
    Unreadable,
}

pub(super) fn claims(entry: &Path, reference: &str) -> Claim {
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
pub(super) fn read_entry(
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
    // The entry states the contract it was prepared under, and admission uses that rather than
    // a fixed one, so a host that prepared a newer machine admits it as the machine it is.
    let profile =
        profile_for(installed_policy_version(&store, &id).map_err(|_| PreparedError::Damaged)?)?;
    let admitted =
        admit_installed_generation(&store, &id, &profile).map_err(|_| PreparedError::Damaged)?;
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
