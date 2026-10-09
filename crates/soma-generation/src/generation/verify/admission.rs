//! Admission of a Generation whose bytes an installation already verified.
//!
//! Admission is deliberately not a substitute for content verification. It rechecks the small
//! content-addressed manifest against the identity and the compiler profile, and it opens every
//! launch artifact by handle at its exact declared size without re-hashing the bytes. What makes
//! that safe is the contract around it: only an installer holds write authority over the store,
//! and the handles returned here are the handles launch must consume, so no later path lookup
//! can substitute launch bytes.

use std::{fs::File, path::Path};

use soma::GenerationId;

use super::super::{
    artifacts::ArtifactDescriptor,
    error::CompileError,
    identity::derive_generation_id,
    manifest::{GenerationManifest, SnapshotBinding, decode_manifest},
    publish::read_manifest_bytes,
    request::CompilerProfile,
};
use super::{from_import, integrity, require_profile};
use crate::{ImportPhase, store::Store};

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
/// This is not a substitute for [`verify_generation`](super::verify_generation).
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

/// The compiler-policy version one canonical ready-manifest byte sequence declares.
///
/// A host that prepared a Generation cannot be asked which machine contract it holds before it
/// reads the manifest: the manifest is what names it. This reads that one field out of hostile
/// bytes under the same bounded decoder the admission path uses, so a caller can resolve the
/// exact compiler profile and then admit the Generation as the machine it was certified as
/// rather than as the one the caller assumed.
///
/// # Errors
///
/// Returns the decoder's failure for bytes that are not one canonical ready manifest.
pub fn declared_policy_version(manifest_bytes: &[u8]) -> Result<u16, CompileError> {
    Ok(decode_manifest(manifest_bytes)?.compiler_policy_version)
}

/// The compiler-policy version the ready manifest for `id` in `store` declares.
///
/// # Errors
///
/// Returns the store, read, or decoder failure for an identity that names no readable manifest.
pub fn installed_policy_version(store: &Path, id: &GenerationId) -> Result<u16, CompileError> {
    let store = Store::open(store).map_err(from_import)?;
    let bytes = read_manifest_bytes(&store, id)?;
    declared_policy_version(&bytes)
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

#[cfg(test)]
mod tests;
