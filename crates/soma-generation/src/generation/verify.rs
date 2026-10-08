use std::{fs::File, io::Read as _, path::Path};

use soma::GenerationId;

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole},
    candidate::CandidateId,
    error::{CompileError, CompileErrorKind, CompilePhase},
    identity::derive_generation_id,
    manifest::{GenerationManifest, SnapshotBinding, decode_manifest},
    overlay::{derive_overlay_hash_seed, derive_overlay_uuid},
    publish::read_manifest_bytes,
    request::CompilerProfile,
};
use crate::{ImportPhase, store::Store};

mod incompatibility;
mod machine;
mod profile;

pub use incompatibility::Incompatibility;
use profile::require_profile;

const MAX_TREE_MANIFEST_BYTES: u64 = 512 * 1024 * 1024;
const EXT4_MAGIC: u16 = 0xEF53;

/// One published Generation whose manifest and every referenced artifact re-verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedGeneration {
    /// The verified identity.
    pub id: GenerationId,
    /// The decoded manifest.
    pub manifest: GenerationManifest,
    /// The number of artifact objects whose size and digest were re-checked.
    pub artifacts_verified: u32,
    /// Whether a certified snapshot is bound; `false` means Launch must refuse it.
    pub launchable: bool,
}

/// One published Candidate whose manifest and every referenced artifact re-verified.
///
/// There is deliberately no `launchable` field: a Candidate is never launchable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCandidate {
    /// The verified Candidate identity.
    pub id: CandidateId,
    /// The decoded manifest.
    pub manifest: GenerationManifest,
    /// The number of artifact objects whose size and digest were re-checked.
    pub artifacts_verified: u32,
}

/// One launchable Generation admitted from a store that installation already verified.
///
/// Installation owns expensive content verification.
/// Admission rechecks the small content-addressed manifest and compiler contract, then retains
/// open handles to every exact artifact so no later path lookup can substitute launch bytes.
#[derive(Debug)]
pub struct InstalledGeneration {
    /// The admitted identity.
    pub id: GenerationId,
    /// The decoded ready manifest.
    pub manifest: GenerationManifest,
    artifacts: Vec<(ArtifactDescriptor, File)>,
}

impl InstalledGeneration {
    /// Consumes the admission result into its manifest and verified-use artifact handles.
    #[must_use]
    pub fn into_parts(self) -> (GenerationManifest, Vec<(ArtifactDescriptor, File)>) {
        (self.manifest, self.artifacts)
    }
}

/// Admits a Generation from a private, installation-verified store without re-reading large
/// artifacts.
///
/// The ready manifest is digest-checked against `id`, decoded as hostile input, and checked
/// against the compiler profile.
/// Every launch artifact is opened without following a final symlink and must have the exact
/// declared size.
/// The returned handles are the handles launch must consume.
///
/// This is not a substitute for [`verify_generation`].
/// Operators must run full verification before publishing `generation.id`, and only the
/// installer may hold write authority over the store.
///
/// # Errors
///
/// Returns the first identity, profile, launchability, or artifact-open failure.
pub fn admit_installed_generation(
    store: &Path,
    id: &GenerationId,
    profile: &CompilerProfile,
) -> Result<InstalledGeneration, CompileError> {
    profile.validate()?;
    let store = Store::open(store).map_err(from_import)?;
    let bytes = read_manifest_bytes(&store, id)?;
    let manifest = decode_manifest(&bytes)?;
    require_profile(&manifest, profile)?;
    if manifest.snapshot == SnapshotBinding::Absent {
        return Err(integrity());
    }
    let artifacts = launch_descriptors(&manifest)
        .into_iter()
        .map(|descriptor| {
            let file = store
                .open_verified_blob(
                    &descriptor.to_store_descriptor(),
                    descriptor.size,
                    ImportPhase::Publish,
                )
                .map_err(from_import)?
                .into_std();
            Ok((descriptor, file))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(InstalledGeneration {
        id: id.clone(),
        manifest,
        artifacts,
    })
}

/// Reconstructs one admitted Generation from canonical manifest bytes and files transferred by
/// a process that already completed [`admit_installed_generation`].
///
/// The process boundary must transfer the open file descriptions themselves, not paths.
/// This function revalidates the manifest identity, hostile decoder, compiler profile, artifact
/// order, file kind, and size without re-hashing bytes held by those already verified handles.
///
/// # Errors
///
/// Returns an integrity failure when the handoff is incomplete or inconsistent.
pub fn admit_verified_handoff(
    id: &GenerationId,
    manifest_bytes: &[u8],
    files: Vec<File>,
    profile: &CompilerProfile,
) -> Result<InstalledGeneration, CompileError> {
    profile.validate()?;
    if &derive_generation_id(manifest_bytes) != id {
        return Err(integrity());
    }
    let manifest = decode_manifest(manifest_bytes)?;
    require_profile(&manifest, profile)?;
    if manifest.snapshot == SnapshotBinding::Absent {
        return Err(integrity());
    }
    let descriptors = launch_descriptors(&manifest);
    if descriptors.len() != files.len() {
        return Err(integrity());
    }
    let artifacts = descriptors
        .into_iter()
        .zip(files)
        .map(|(descriptor, file)| {
            let metadata = file.metadata().map_err(|_| integrity())?;
            if !metadata.file_type().is_file() || metadata.len() != descriptor.size {
                return Err(integrity());
            }
            Ok((descriptor, file))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(InstalledGeneration {
        id: id.clone(),
        manifest,
        artifacts,
    })
}

#[cfg(test)]
mod installed_admission_tests;

mod generation;

pub use generation::{verify_candidate, verify_generation};

fn launch_descriptors(manifest: &GenerationManifest) -> Vec<ArtifactDescriptor> {
    let mut descriptors = vec![
        manifest.kernel.descriptor,
        manifest.initramfs.descriptor,
        manifest.root.descriptor,
    ];
    descriptors.extend(
        manifest
            .overlay
            .templates
            .iter()
            .map(|template| template.descriptor),
    );
    if let SnapshotBinding::Captured {
        memory,
        overlay,
        state,
        ..
    } = manifest.snapshot
    {
        descriptors.extend([memory, overlay, state]);
    }
    descriptors.sort_by_key(|descriptor| *descriptor.digest.as_bytes());
    descriptors.dedup();
    descriptors
}

fn verify_ext4_superblock(raw: &[u8], capacity: u64) -> Result<(), CompileError> {
    let u16_at = |offset: usize| u16::from_le_bytes([raw[offset], raw[offset + 1]]);
    let u32_at = |offset: usize| {
        u32::from_le_bytes([
            raw[offset],
            raw[offset + 1],
            raw[offset + 2],
            raw[offset + 3],
        ])
    };
    let block_count = u64::from(u32_at(0x04)) | (u64::from(u32_at(0x150)) << 32);
    let block_size = 1024_u64 << u32_at(0x18);
    let mut label = [0_u8; 16];
    label[..super::overlay::OVERLAY_VOLUME_LABEL.len()]
        .copy_from_slice(super::overlay::OVERLAY_VOLUME_LABEL.as_bytes());
    if u16_at(0x38) != EXT4_MAGIC
        || raw[0x68..0x78] != derive_overlay_uuid(capacity)
        || raw[0x78..0x88] != label
        || raw[0xec..0xfc] != derive_overlay_hash_seed(capacity)
        || u16_at(0x58) != 256
        || block_size != 4096
        || block_count.checked_mul(block_size) != Some(capacity)
    {
        return Err(integrity());
    }
    Ok(())
}

fn read_artifact(
    store: &Store,
    descriptor: &ArtifactDescriptor,
    maximum: u64,
) -> Result<Vec<u8>, CompileError> {
    if descriptor.role == ArtifactRole::ErofsRoot {
        return Err(integrity());
    }
    let mut file = store
        .open_verified_blob(
            &descriptor.to_store_descriptor(),
            maximum,
            ImportPhase::Publish,
        )
        .map_err(from_import)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| io_error())?;
    Ok(bytes)
}

fn from_import(error: crate::ImportError) -> CompileError {
    CompileError::from_import(CompilePhase::VerifyGeneration, error)
}

const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::VerifyGeneration, CompileErrorKind::Integrity)
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::VerifyGeneration, CompileErrorKind::Io)
}

#[cfg(test)]
mod tests;
