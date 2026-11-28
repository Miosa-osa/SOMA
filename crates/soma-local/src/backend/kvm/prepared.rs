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

use std::{path::PathBuf, sync::Arc};

use soma::GenerationId;
use soma_generation::{ArtifactDescriptor, GenerationManifest, generation_manifest};

mod cache;
mod entry;
mod prune;
mod scan;

pub(super) use entry::from_handoff;
pub(crate) use prune::prune;
pub(crate) use scan::catalog;
#[cfg(test)]
use scan::catalog_in;
#[cfg(test)]
#[path = "prepared_catalog_tests.rs"]
mod catalog_tests;
pub(super) use scan::{find, preload, store_root};

/// Names the root holding Generations prepared for this host.
pub(super) const STORE: &str = "SOMA_GENERATION_STORE";

/// The ready Generation identity published only after certification succeeds.
const GENERATION_ID: &str = "generation.id";
/// The non-launchable build result retained for diagnostics and later certification.
const CANDIDATE: &str = "candidate.somacan";
/// The image reference this Generation was prepared for, with no trailing newline.
const REFERENCE: &str = "reference";
/// The artifact store the Candidate manifest describes.
const STORE_DIRECTORY: &str = "store";
/// Private publisher staging, outside the runtime and prune entry sets.
const INCOMING: &str = ".incoming";

/// Most a reference file may hold. An image reference is short.
const MAX_REFERENCE_BYTES: u64 = 4096;
/// Exact upper bound for `sha256:` plus 64 lowercase hexadecimal digits.
const GENERATION_ID_BYTES: u64 = 72;

type VerifiedArtifact = (ArtifactDescriptor, Arc<std::fs::File>);
type CacheKey = (PathBuf, String);

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
    /// The entry holds only a Candidate and therefore cannot be launched.
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
    /// The identity of the independently re-verified ready Generation.
    pub(super) id: GenerationId,
    /// The decoded ready Generation manifest.
    pub(super) manifest: GenerationManifest,
    /// Open handles to the exact artifact bytes verified while this entry was admitted.
    ///
    /// Launch opens independent descriptions of these handles rather than looking digest names
    /// up again.
    /// This makes verification an admission cost and prevents a path replacement after
    /// verification from changing the bytes a later machine consumes.
    artifacts: Vec<VerifiedArtifact>,
    /// The stamp each verified artifact's content was hashed or ledger-matched under.
    ///
    /// A resident restore hashes the root; this lets it skip that read for a root whose stamp
    /// has not moved since its content was verified. An artifact with no such stamp is absent.
    admitted_stamps: Vec<(ArtifactDescriptor, soma_kvm::snapshot::FileStamp)>,
}

impl PreparedGeneration {
    /// Opens an independent description of an already verified artifact for one launch.
    pub(super) fn open_artifact(
        &self,
        descriptor: &ArtifactDescriptor,
    ) -> Result<std::fs::File, PreparedError> {
        let file = self
            .artifacts
            .iter()
            .find(|(candidate, _)| candidate == descriptor)
            .map(|(_, file)| file)
            .ok_or(PreparedError::Damaged)?;
        independent_description(file).map_err(|_| PreparedError::Damaged)
    }

    /// The broker's attestation of one admitted artifact at the stamp it had when admitted.
    pub(super) fn attestation(
        &self,
        descriptor: &ArtifactDescriptor,
    ) -> Option<soma_kvm::snapshot::ArtifactAttestation> {
        let (_, stamp) = self
            .admitted_stamps
            .iter()
            .find(|(candidate, _)| candidate == descriptor)?;
        let (_, file) = self
            .artifacts
            .iter()
            .find(|(candidate, _)| candidate == descriptor)?;
        if !super::checkpoint::stamps_hold(file) {
            return None;
        }
        let digest = soma_kvm::snapshot::Digest::from_bytes(*descriptor.digest.as_bytes());
        Some(soma_kvm::snapshot::ArtifactAttestation::new(digest, *stamp))
    }

    /// The stamp the root's content was verified under, for a child this entry hands off to.
    pub(super) fn root_stamp(&self) -> Option<soma_kvm::snapshot::FileStamp> {
        self.admitted_stamps
            .iter()
            .find(|(descriptor, _)| descriptor == &self.manifest.root.descriptor)
            .map(|(_, stamp)| *stamp)
    }

    /// Encodes the admitted manifest and opens independent verified handles for one child.
    pub(super) fn handoff(&self) -> Result<(Vec<u8>, Vec<std::fs::File>), PreparedError> {
        let manifest = generation_manifest::encode_manifest(&self.manifest)
            .map_err(|_| PreparedError::Damaged)?;
        let artifacts = self
            .artifacts
            .iter()
            .map(|(_, file)| independent_description(file).map_err(|_| PreparedError::Damaged))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((manifest, artifacts))
    }
}

/// Opens the inode retained by `file` as a new open file description.
///
/// `File::try_clone` and `SCM_RIGHTS` both duplicate one open file description, including its
/// mutable offset. Kernel, initramfs, and snapshot readers use sequential reads, so sharing that
/// offset across concurrent launches lets one child move another child's cursor or leave it at
/// EOF. Opening this process's retained descriptor through procfs names the already open inode,
/// not the replaceable installed path, while giving every launch an independent cursor.
#[cfg(target_os = "linux")]
fn independent_description(file: &std::fs::File) -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd as _;

    std::fs::File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// Non-Linux clients cannot launch the KVM backend, but the portable crate still compiles there.
#[cfg(not(target_os = "linux"))]
fn independent_description(file: &std::fs::File) -> std::io::Result<std::fs::File> {
    file.try_clone()
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
