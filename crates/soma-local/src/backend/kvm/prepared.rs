//! Generations prepared ahead of demand, and how a request finds one.
//!
//! The request path must not acquire an OCI image or construct a Generation: preparation happens
//! before demand, and a request either finds a prepared Generation or is refused. That is why
//! this module only ever reads.
//!
//! A prepared root holds one directory per Generation. Each carries the exact published Candidate
//! bytes, the artifact store those bytes describe, and the image reference it was prepared for.
//! Identity is recomputed from the bytes on every read rather than recorded beside them, so a
//! tampered or truncated entry cannot present itself as a Generation it is not.

use std::path::{Path, PathBuf};

use soma_generation::{CandidateId, GenerationManifest, generation_manifest::decode_candidate};

/// Names the root holding Generations prepared for this host.
pub(super) const STORE: &str = "SOMA_GENERATION_STORE";

/// The exact published Candidate bytes, from which identity and manifest are recovered.
const CANDIDATE: &str = "candidate.somacan";
/// The image reference this Generation was prepared for, with no trailing newline.
const REFERENCE: &str = "reference";
/// The artifact store the Candidate manifest describes.
const STORE_DIRECTORY: &str = "store";

/// Why a request cannot be served from the prepared root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PreparedError {
    /// `SOMA_GENERATION_STORE` is unset, so this host prepares nothing.
    StoreUnset,
    /// The named root does not exist or cannot be read.
    StoreUnreadable,
    /// The root is readable and holds no Generation prepared for this reference.
    NotPrepared,
    /// More than one entry claims this reference, so which one launches is undefined.
    Ambiguous,
    /// An entry, or a file inside it, is a symbolic link.
    ///
    /// A link means the bytes that launch can be redirected after they were verified, so a
    /// linked entry is refused rather than followed.
    Linked,
    /// An entry matched the reference but its bytes could not be decoded.
    ///
    /// This is kept distinct from [`Self::NotPrepared`] because a damaged entry is an operator
    /// fault on a host that believes it is prepared, not an ordinary miss.
    Damaged,
}

/// One Generation that a host prepared before any request asked for it.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "the store and identity are read by launch, which still fails closed"
)]
pub(super) struct PreparedGeneration {
    /// The artifact store holding the root, overlay template, kernel, and agent.
    pub(super) store: PathBuf,
    /// The Candidate identity, recomputed from the exact published bytes.
    pub(super) id: CandidateId,
    /// The decoded Candidate manifest.
    pub(super) manifest: GenerationManifest,
}

/// Whether `path` is a symbolic link, treating an unreadable path as one.
///
/// `symlink_metadata` does not follow the final component, so this reports the link itself
/// rather than what it points at. A path that cannot be read at all is refused for the same
/// reason a link is: what launches must be exactly what was verified.
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Whether this entry claims `reference`.
///
/// Claiming is decided by the reference text alone, before anything else is read, so that two
/// entries claiming one reference are ambiguous whatever their contents are.
fn claims(entry: &Path, reference: &str) -> bool {
    std::fs::read_to_string(entry.join(REFERENCE))
        .is_ok_and(|prepared_for| prepared_for.trim() == reference)
}

/// Reads the one entry that claims the reference, once it is known to be the only one.
fn read_entry(entry: &Path) -> Result<PreparedGeneration, PreparedError> {
    let candidate = entry.join(CANDIDATE);
    let store = entry.join(STORE_DIRECTORY);
    if is_link(entry) || is_link(&entry.join(REFERENCE)) || is_link(&candidate) || is_link(&store) {
        return Err(PreparedError::Linked);
    }
    let bytes = std::fs::read(&candidate).map_err(|_| PreparedError::Damaged)?;
    let manifest = decode_candidate(&bytes).map_err(|_| PreparedError::Damaged)?;
    if !store.is_dir() {
        return Err(PreparedError::Damaged);
    }
    Ok(PreparedGeneration {
        store,
        id: CandidateId::of(&bytes),
        manifest,
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
pub(super) fn find(
    root: Option<&Path>,
    reference: &str,
) -> Result<PreparedGeneration, PreparedError> {
    let root = root.ok_or(PreparedError::StoreUnset)?;
    if is_link(root) {
        return Err(PreparedError::Linked);
    }
    let entries = std::fs::read_dir(root).map_err(|_| PreparedError::StoreUnreadable)?;
    let mut claimants = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && claims(&path, reference) {
            claimants.push(path);
        }
    }
    match claimants.as_slice() {
        [] => Err(PreparedError::NotPrepared),
        [only] => read_entry(only),
        _ => Err(PreparedError::Ambiguous),
    }
}

/// The prepared root this host names, if any.
pub(super) fn store_root() -> Option<PathBuf> {
    std::env::var_os(STORE).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unnamed_store_prepares_nothing() {
        assert_eq!(
            find(None, "node:22").expect_err("an unset store must refuse"),
            PreparedError::StoreUnset
        );
    }

    #[test]
    fn a_missing_root_is_distinguished_from_an_empty_one() {
        assert_eq!(
            find(Some(Path::new("/nonexistent/soma-generations")), "node:22")
                .expect_err("a missing root must refuse"),
            PreparedError::StoreUnreadable
        );
    }

    #[test]
    fn a_readable_root_without_a_match_is_not_prepared() {
        let root = std::env::temp_dir().join(format!("soma-prepared-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create the test root");
        let found = find(Some(&root), "node:22");
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(
            found.expect_err("an empty root must refuse"),
            PreparedError::NotPrepared
        );
    }

    #[test]
    fn an_entry_matching_the_reference_with_unreadable_bytes_is_damaged_not_missing() {
        let root =
            std::env::temp_dir().join(format!("soma-prepared-damaged-{}", std::process::id()));
        let entry = root.join("one");
        std::fs::create_dir_all(&entry).expect("create the test entry");
        std::fs::write(entry.join(REFERENCE), "node:22\n").expect("write the reference");
        // The Candidate bytes are absent, so the entry claims a reference it cannot serve.
        let found = find(Some(&root), "node:22");
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(
            found.expect_err("a damaged entry must refuse"),
            PreparedError::Damaged
        );
    }

    /// Two entries claiming one reference must not resolve by directory order.
    #[test]
    fn duplicate_references_are_ambiguous_rather_than_first_wins() {
        let root = std::env::temp_dir().join(format!("soma-prepared-dup-{}", std::process::id()));
        for name in ["one", "two"] {
            let entry = root.join(name);
            std::fs::create_dir_all(entry.join(STORE_DIRECTORY)).expect("create the entry");
            std::fs::write(entry.join(REFERENCE), "node:22").expect("write the reference");
            std::fs::write(entry.join(CANDIDATE), b"not a candidate").expect("write bytes");
        }
        let found = find(Some(&root), "node:22");
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(
            found.expect_err("two entries for one reference must refuse"),
            PreparedError::Ambiguous
        );
    }

    /// A linked entry could be redirected after it was verified.
    #[test]
    fn a_symlinked_entry_is_refused_rather_than_followed() {
        let root = std::env::temp_dir().join(format!("soma-prepared-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create the root");
        // The entry that actually holds the bytes lives outside the prepared root, so the only
        // way this reference can resolve is by following the link, which is what must not
        // happen: a link can be repointed after the entry it named was verified.
        let outside = std::env::temp_dir().join(format!("soma-target-{}", std::process::id()));
        std::fs::create_dir_all(outside.join(STORE_DIRECTORY)).expect("create the target");
        std::fs::write(outside.join(REFERENCE), "node:22").expect("write the reference");
        std::fs::write(outside.join(CANDIDATE), b"not a candidate").expect("write bytes");
        std::os::unix::fs::symlink(&outside, root.join("linked")).expect("link the entry");

        let found = find(Some(&root), "node:22");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
        assert_eq!(
            found.expect_err("a linked entry must refuse"),
            PreparedError::Linked
        );
    }

    #[test]
    fn an_entry_prepared_for_another_reference_is_skipped_rather_than_read() {
        let root = std::env::temp_dir().join(format!("soma-prepared-other-{}", std::process::id()));
        let entry = root.join("one");
        std::fs::create_dir_all(&entry).expect("create the test entry");
        std::fs::write(entry.join(REFERENCE), "alpine:3.20").expect("write the reference");
        // No Candidate bytes: reaching them would be Damaged, so NotPrepared proves the
        // reference is checked before anything else is read.
        let found = find(Some(&root), "node:22");
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(
            found.expect_err("a non-matching entry must not match"),
            PreparedError::NotPrepared
        );
    }
}
