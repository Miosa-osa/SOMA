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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use soma_generation::{CandidateId, GenerationManifest, generation_manifest::decode_candidate};

/// Names the root holding Generations prepared for this host.
pub(super) const STORE: &str = "SOMA_GENERATION_STORE";

/// Opts this host into launching uncertified Candidates.
///
/// Certification does not exist yet, so every Generation a host can prepare today is a
/// Candidate that no gate has verified. Launching one is a development and diagnostic
/// behaviour, not a production one, and it must not be reachable by accident: without this
/// variable set to `1`, a Candidate is refused before any machine is created.
pub(super) const ALLOW_UNCERTIFIED: &str = "SOMA_ALLOW_UNCERTIFIED_GENERATION";

/// The exact published Candidate bytes, from which identity and manifest are recovered.
const CANDIDATE: &str = "candidate.somacan";
/// The image reference this Generation was prepared for, with no trailing newline.
const REFERENCE: &str = "reference";
/// The artifact store the Candidate manifest describes.
const STORE_DIRECTORY: &str = "store";

/// Most a reference file may hold. An image reference is short.
const MAX_REFERENCE_BYTES: u64 = 4096;
/// Most a published Candidate may hold, matching the manifest size the encoder admits.
const MAX_CANDIDATE_BYTES: u64 = 1 << 20;

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
    /// The entry holds a Candidate and this host has not opted into launching one.
    Uncertified,
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
    /// The image reference this entry claims.
    ///
    /// A machine host finds its own entry from this rather than being handed a store path, so
    /// what it launches is what a prepared entry claims rather than bytes a caller named.
    pub(super) reference: String,
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
fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
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
    let candidate = entry.join(CANDIDATE);
    let store = entry.join(STORE_DIRECTORY);
    if any_component_is_link(root, entry)
        || is_link(&entry.join(REFERENCE))
        || is_link(&candidate)
        || is_link(&store)
    {
        return Err(PreparedError::Linked);
    }
    let bytes = read_bounded(&candidate, MAX_CANDIDATE_BYTES).ok_or(PreparedError::Damaged)?;
    let manifest = decode_candidate(&bytes).map_err(|_| PreparedError::Damaged)?;
    if !store.is_dir() {
        return Err(PreparedError::Damaged);
    }
    Ok(PreparedGeneration {
        store,
        reference: reference.to_owned(),
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
    allow_uncertified: bool,
) -> Result<PreparedGeneration, PreparedError> {
    let root = root.ok_or(PreparedError::StoreUnset)?;
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
    match claimants.as_slice() {
        [] => Err(PreparedError::NotPrepared),
        // Nothing a host can prepare today carries a certification, so a Candidate is stopped
        // here, before it is decoded and before any overlay, machine, vCPU, or guest thread
        // exists. Refusing before the decode also means a damaged entry is still refused as
        // uncertified rather than reporting how it was damaged.
        [_] if !allow_uncertified => Err(PreparedError::Uncertified),
        [only] => read_entry(root, only, reference),
        _ => Err(PreparedError::Ambiguous),
    }
}

/// Whether this host opted into launching an uncertified Candidate.
pub(super) fn uncertified_allowed() -> bool {
    allows_uncertified(std::env::var_os(ALLOW_UNCERTIFIED).as_deref())
}

/// The opt-in rule, over an already-read value so it is testable without touching the process.
///
/// Only the exact value `1` opts in. Any other setting, including an empty value or the word
/// `true`, leaves the host refusing Candidates, because a half-recognised setting must not be
/// the difference between refusing and launching unverified bytes.
fn allows_uncertified(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// The prepared root this host names, if any.
pub(super) fn store_root() -> Option<PathBuf> {
    std::env::var_os(STORE).map(PathBuf::from)
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
