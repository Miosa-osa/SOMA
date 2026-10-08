//! Re-verifying a published Generation or Candidate across every artifact it references.
//!
//! Verification is the hostile-input boundary: the manifest bytes are re-hashed against the
//! identity and decoded rather than trusted, every descriptor is reopened from the store with
//! its exact size and digest, and each artifact is re-parsed against the binding that names it.

use std::{io::Read as _, path::Path};

use soma::GenerationId;

use super::super::{
    candidate::CandidateId,
    certify::verify_snapshot_binding,
    erofs,
    erofs_reader::ErofsImage,
    erofs_verify::{RootExpectation, verify_root_image},
    error::{CompileError, CompilePhase},
    initramfs::verify_initramfs,
    kernel::verify_kernel,
    manifest::{GenerationManifest, SnapshotBinding, decode_candidate, decode_manifest},
    publish::{read_candidate_bytes, read_manifest_bytes},
    request::CompilerProfile,
};
use super::{
    MAX_TREE_MANIFEST_BYTES, VerifiedCandidate, VerifiedGeneration, from_import, integrity,
    io_error, read_artifact, require_profile, verify_ext4_superblock,
};
use crate::{ImportPhase, normalize::TREE_MEDIA_TYPE, oci::Descriptor, store::Store};

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
        || contents.layout_version != manifest.initramfs.layout_version
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
