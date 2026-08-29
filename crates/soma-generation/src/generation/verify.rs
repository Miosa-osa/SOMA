use std::{io::Read as _, path::Path};

use soma::GenerationId;

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole},
    contracts,
    erofs::{self, derive_root_uuid},
    erofs_reader::ErofsImage,
    erofs_verify::{RootExpectation, verify_root_image},
    error::{CompileError, CompileErrorKind, CompilePhase},
    initramfs::{INITRAMFS_LAYOUT_VERSION, verify_initramfs},
    kernel::{ELF_PVH_CONTRACT_VERSION, verify_kernel},
    manifest::{GenerationManifest, SnapshotBinding, decode_manifest},
    overlay::{OVERLAY_UUID_DERIVATION_VERSION, derive_overlay_hash_seed, derive_overlay_uuid},
    publish::read_manifest_bytes,
    request::CompilerProfile,
};
use crate::{ImportPhase, normalize::TREE_MEDIA_TYPE, oci::Descriptor, store::Store};

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
/// Returns the first failing phase and kind; a bound snapshot returns
/// [`CompileErrorKind::Unimplemented`] because no snapshot verifier exists yet.
pub fn verify_generation(
    store: &Path,
    id: &GenerationId,
    profile: &CompilerProfile,
) -> Result<VerifiedGeneration, CompileError> {
    profile.validate()?;
    let store = Store::open(store).map_err(from_import)?;
    let bytes = read_manifest_bytes(&store, id)?;
    let manifest = decode_manifest(&bytes)?;
    require_profile(&manifest, profile)?;
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
    let kernel = read_artifact(
        &store,
        &manifest.kernel.descriptor,
        profile.max_kernel_bytes,
    )?;
    let kernel = verify_kernel(&kernel)?;
    if kernel.digest != manifest.kernel.descriptor.digest {
        return Err(integrity());
    }
    let initramfs = read_artifact(
        &store,
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
    let launchable = match manifest.snapshot {
        SnapshotBinding::Absent => false,
        SnapshotBinding::Captured { .. } => {
            return Err(CompileError::new(
                CompilePhase::VerifyGeneration,
                CompileErrorKind::Unimplemented,
            ));
        }
    };
    Ok(VerifiedGeneration {
        id: id.clone(),
        manifest,
        artifacts_verified: verified,
        launchable,
    })
}

fn require_profile(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    let root_uuid = derive_root_uuid(&manifest.tree.digest);
    let expected_features = super::overlay::overlay_feature_profile();
    if manifest.compiler_policy_version != profile.policy_version
        || manifest.root.uuid != root_uuid
        || manifest.root.format_profile != erofs::EROFS_FORMAT_PROFILE
        || manifest.root.formatter_revision != erofs::EROFS_UTILS_REVISION
        || manifest.overlay.uuid_derivation_version != OVERLAY_UUID_DERIVATION_VERSION
        || manifest.overlay.feature_profile != expected_features
        || manifest.overlay.templates.is_empty()
        || manifest.kernel.elf_pvh_contract_version != ELF_PVH_CONTRACT_VERSION
        || manifest.kernel.cpu_architecture != "x86_64"
        || manifest.initramfs.layout_version != INITRAMFS_LAYOUT_VERSION
        || manifest.command_line != contracts::kernel_command_line_v1()
        || manifest.machine_contract != contracts::machine_contract_v1()
        || manifest.device_contract != contracts::device_contract_v1()
        || manifest.cpu_template != contracts::cpu_template_v1()
        || manifest.repair.readiness_command_digest != contracts::readiness_command_digest()
        || manifest.shape.vcpu_count != 1
    {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            CompileErrorKind::Unsupported,
        ));
    }
    Ok(())
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
