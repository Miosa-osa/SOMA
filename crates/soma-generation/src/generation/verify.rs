use std::{fs::File, io::Read as _, path::Path};

use soma::GenerationId;

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole},
    candidate::CandidateId,
    certify::verify_snapshot_binding,
    erofs::{self},
    erofs_reader::ErofsImage,
    erofs_verify::{RootExpectation, verify_root_image},
    error::{CompileError, CompileErrorKind, CompilePhase},
    identity::derive_generation_id,
    initramfs::verify_initramfs,
    kernel::verify_kernel,
    manifest::{GenerationManifest, SnapshotBinding, decode_candidate, decode_manifest},
    overlay::{derive_overlay_hash_seed, derive_overlay_uuid},
    publish::{read_candidate_bytes, read_manifest_bytes},
    request::CompilerProfile,
};
use crate::{ImportPhase, normalize::TREE_MEDIA_TYPE, oci::Descriptor, store::Store};

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
mod installed_admission_tests {
    use std::{fs, io::Cursor};

    use super::*;
    use crate::{
        digest,
        generation::{
            artifacts::Sha256Digest,
            identity::derive_generation_id,
            manifest::{encode_manifest, fixture},
        },
    };

    const BLOCK: u64 = 4096;
    const OVERLAY: u64 = 64 * 1024 * 1024;

    fn resize(descriptor: &mut ArtifactDescriptor, size: u64) {
        let fill = descriptor.role.code();
        let bytes = vec![fill; usize::try_from(size).unwrap()];
        descriptor.size = size;
        descriptor.digest = Sha256Digest::from_oci(&digest::bytes(&bytes));
    }

    fn installed() -> (
        tempfile::TempDir,
        GenerationId,
        CompilerProfile,
        Vec<ArtifactDescriptor>,
    ) {
        let root = tempfile::tempdir().expect("store root");
        let store = Store::open(root.path()).expect("open store");
        let mut manifest = fixture::profile_v1();
        manifest.overlay.templates.truncate(1);
        manifest.overlay.templates[0].capacity = OVERLAY;
        manifest.overlay.minimum_capacity = OVERLAY;
        manifest.overlay.maximum_capacity = OVERLAY;
        manifest.template.writable_storage_bytes = OVERLAY;
        manifest.snapshot = fixture::captured_snapshot();
        resize(&mut manifest.kernel.descriptor, BLOCK);
        resize(&mut manifest.initramfs.descriptor, BLOCK);
        resize(&mut manifest.root.descriptor, BLOCK);
        resize(&mut manifest.overlay.templates[0].descriptor, OVERLAY);
        if let SnapshotBinding::Captured {
            memory,
            overlay,
            state,
            ..
        } = &mut manifest.snapshot
        {
            resize(memory, BLOCK);
            resize(overlay, BLOCK);
            resize(state, BLOCK);
        }
        let descriptors = launch_descriptors(&manifest);
        for descriptor in &descriptors {
            let bytes = vec![descriptor.role.code(); usize::try_from(descriptor.size).unwrap()];
            store
                .put_descriptor(
                    &mut Cursor::new(bytes),
                    &descriptor.to_store_descriptor(),
                    descriptor.size,
                    ImportPhase::Publish,
                )
                .expect("publish artifact");
        }
        let bytes = encode_manifest(&manifest).expect("encode ready manifest");
        store
            .put_bytes(
                &bytes,
                ArtifactRole::GenerationManifest.media_type(),
                ImportPhase::Publish,
            )
            .expect("publish ready manifest");
        let mut profile = CompilerProfile::v1();
        profile.overlay_capacities = vec![OVERLAY];
        (root, derive_generation_id(&bytes), profile, descriptors)
    }

    #[test]
    fn admission_retains_every_digest_verified_launch_handle() {
        let (root, id, profile, descriptors) = installed();
        let admitted = admit_installed_generation(root.path(), &id, &profile).expect("admit");

        assert_eq!(admitted.artifacts.len(), descriptors.len());
    }

    #[test]
    fn same_size_corruption_is_refused() {
        let (root, id, profile, descriptors) = installed();
        let target = &descriptors[0];
        let path = root
            .path()
            .join("v1/blobs/sha256")
            .join(crate::digest::hex(&target.digest.to_oci()));
        fs::remove_file(&path).expect("remove verified artifact");
        fs::write(&path, vec![0xff; usize::try_from(target.size).unwrap()])
            .expect("replace with same-size corruption");

        assert!(admit_installed_generation(root.path(), &id, &profile).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_artifact_is_refused() {
        let (root, id, profile, descriptors) = installed();
        let target = &descriptors[0];
        let path = root
            .path()
            .join("v1/blobs/sha256")
            .join(crate::digest::hex(&target.digest.to_oci()));
        let substitute = root.path().join("same-size-substitute");
        fs::write(
            &substitute,
            vec![0_u8; usize::try_from(target.size).unwrap()],
        )
        .expect("write substitute");
        fs::remove_file(&path).expect("remove artifact");
        std::os::unix::fs::symlink(&substitute, &path).expect("replace with symlink");

        assert!(admit_installed_generation(root.path(), &id, &profile).is_err());
    }

    #[test]
    fn a_manifest_without_a_snapshot_is_never_admitted() {
        let root = tempfile::tempdir().expect("store root");
        let store = Store::open(root.path()).expect("open store");
        let manifest = fixture::profile_v1();
        let bytes = encode_manifest(&manifest).expect("encode manifest");
        store
            .put_bytes(
                &bytes,
                ArtifactRole::GenerationManifest.media_type(),
                ImportPhase::Publish,
            )
            .expect("publish manifest");

        let id = derive_generation_id(&bytes);
        assert!(admit_installed_generation(root.path(), &id, &CompilerProfile::v1()).is_err());
    }
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

/// Re-verifies a published Generation across all of its artifacts.
///
/// The manifest bytes are re-hashed against the identity and decoded as hostile input.
/// Every descriptor is reopened from the store with exact size and digest.
/// The kernel is re-parsed, the initramfs re-decoded against its early-init binding, the EROFS
/// image re-walked against the stored tree manifest, and each overlay template's ext4 superblock
/// checked natively for UUID, label, hash seed, block size, and capacity.
/// Contract digests and the command line must equal the profile v1 values.
///
/// # Errors
///
/// Returns the first failing phase and kind.
pub fn verify_generation(
    store: &Path,
    id: &GenerationId,
    profile: &CompilerProfile,
) -> Result<VerifiedGeneration, CompileError> {
    profile.validate()?;
    let store = Store::open(store).map_err(from_import)?;
    let bytes = read_manifest_bytes(&store, id)?;
    let manifest = decode_manifest(&bytes)?;
    // The artifact walk runs before the snapshot decision so a tampered ready manifest is
    // rejected on its content rather than on its shape alone.
    let artifacts_verified = verify_decoded(&store, &manifest, profile)?;
    if manifest.snapshot == SnapshotBinding::Absent {
        // Publishing a ready manifest requires the certification token, and that token carries
        // the snapshot binding, so a ready manifest without one was never produced here.
        return Err(integrity());
    }
    verify_snapshot_binding(
        &store,
        manifest.snapshot,
        None,
        CompilePhase::VerifyGeneration,
    )?;
    Ok(VerifiedGeneration {
        id: id.clone(),
        manifest,
        artifacts_verified,
        launchable: true,
    })
}

/// Re-verifies a published Candidate across all of its artifacts.
///
/// A Candidate is build-time state: this never reports launchability and never accepts a ready
/// Generation manifest.
///
/// # Errors
///
/// Returns the first failing phase and kind.
pub fn verify_candidate(
    store: &Path,
    id: &CandidateId,
    profile: &CompilerProfile,
) -> Result<VerifiedCandidate, CompileError> {
    profile.validate()?;
    let store = Store::open(store).map_err(from_import)?;
    let bytes = read_candidate_bytes(&store, id)?;
    let manifest = decode_candidate(&bytes)?;
    if manifest.snapshot != SnapshotBinding::Absent {
        return Err(integrity());
    }
    let verified = verify_decoded(&store, &manifest, profile)?;
    Ok(VerifiedCandidate {
        id: id.clone(),
        manifest,
        artifacts_verified: verified,
    })
}

fn verify_decoded(
    store: &Store,
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<u32, CompileError> {
    require_profile(manifest, profile)?;
    let mut verified = 0_u32;
    for descriptor in manifest.descriptors() {
        store
            .open_verified_blob(
                &descriptor.to_store_descriptor(),
                descriptor.size,
                ImportPhase::Publish,
            )
            .map_err(from_import)?;
        verified += 1;
    }
    let kernel = read_artifact(store, &manifest.kernel.descriptor, profile.max_kernel_bytes)?;
    let kernel = verify_kernel(&kernel)?;
    if kernel.digest != manifest.kernel.descriptor.digest {
        return Err(integrity());
    }
    let initramfs = read_artifact(
        store,
        &manifest.initramfs.descriptor,
        profile.max_initramfs_bytes,
    )?;
    let contents = verify_initramfs(&initramfs)?;
    if contents.early_init_digest != manifest.initramfs.early_init_digest
        || contents.guest_agent_digest != manifest.guest_agent.descriptor.digest
    {
        return Err(integrity());
    }
    let tree = Descriptor {
        media_type: TREE_MEDIA_TYPE.to_owned(),
        digest: manifest.tree.digest.to_oci(),
        size: manifest.tree.size,
        platform: None,
    };
    let mut tree_bytes = Vec::new();
    store
        .open_verified_blob(&tree, MAX_TREE_MANIFEST_BYTES, ImportPhase::Publish)
        .map_err(from_import)?
        .read_to_end(&mut tree_bytes)
        .map_err(|_| io_error())?;
    let root = store
        .open_verified_blob(
            &manifest.root.descriptor.to_store_descriptor(),
            profile.max_root_bytes,
            ImportPhase::Publish,
        )
        .map_err(from_import)?;
    let expectation = RootExpectation {
        uuid: manifest.root.uuid,
        volume_name: erofs::volume_name(),
        epoch: profile.epoch,
    };
    verify_root_image(
        ErofsImage::from_file(root.into_std(), profile.max_root_bytes)?,
        &tree_bytes,
        profile.tree,
        &expectation,
    )?;
    for template in &manifest.overlay.templates {
        let mut file = store
            .open_blob(
                &template.descriptor.to_store_descriptor(),
                ImportPhase::Publish,
            )
            .map_err(from_import)?;
        let mut superblock = vec![0_u8; 2048];
        file.read_exact(&mut superblock).map_err(|_| io_error())?;
        verify_ext4_superblock(&superblock[1024..], template.capacity)?;
    }
    Ok(verified)
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
