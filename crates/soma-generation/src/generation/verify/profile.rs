//! Host-profile compatibility of one decoded manifest: identity and artifact groups.
//!
//! Every field arrives from bytes a hostile party may have produced, so each is validated
//! before the compiler acts on it, numeric relations use checked arithmetic, and every failure
//! returns one typed redacted [`Incompatibility`].

use crate::generation::{
    artifacts::{ArtifactDescriptor, Sha256Digest},
    erofs::{self, derive_root_uuid},
    error::CompileError,
    initramfs::INITRAMFS_LAYOUT_VERSION,
    kernel::ELF_PVH_CONTRACT_VERSION,
    manifest::GenerationManifest,
    overlay::{OVERLAY_UUID_DERIVATION_VERSION, overlay_feature_profile},
    request::CompilerProfile,
};

use super::{Incompatibility, machine};

/// The EROFS block size every root image is formatted with.
const EROFS_BLOCK_BYTES: u64 = 4096;
/// The smallest certified writable class.
const MINIMUM_OVERLAY_BYTES: u64 = 64 * 1024 * 1024;
/// The alignment every writable class is built on.
const OVERLAY_ALIGNMENT_BYTES: u64 = 4 * 1024 * 1024;
/// The decoder bound on the canonical tree manifest.
const MAX_TREE_MANIFEST_BYTES: u64 = 512 * 1024 * 1024;

/// Rejects a manifest that is not compatible with the exact host profile.
///
/// # Errors
///
/// Returns one typed redacted rejection for the first violated invariant.
pub(super) fn require_profile(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    require(
        manifest.compiler_policy_version == profile.policy_version,
        Incompatibility::PolicyVersion,
    )?;
    require_source(manifest)?;
    require_root(manifest, profile)?;
    require_overlay(manifest, profile)?;
    require_machine_artifacts(manifest, profile)?;
    machine::require_machine(manifest, profile)?;
    require_total_size(manifest)
}

fn require_source(manifest: &GenerationManifest) -> Result<(), CompileError> {
    let platform = &manifest.source.platform;
    require(
        platform.operating_system() == "linux"
            && platform.architecture() == "amd64"
            && platform.variant().is_none(),
        Incompatibility::SourcePlatform,
    )?;
    nonzero(&manifest.source.oci_manifest_digest)?;
    nonzero(&manifest.tree.digest)?;
    require(
        manifest.tree.size > 0 && manifest.tree.size <= MAX_TREE_MANIFEST_BYTES,
        Incompatibility::TreeSize,
    )
}

fn require_root(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    let root = &manifest.root;
    require(
        root.uuid == derive_root_uuid(&manifest.tree.digest),
        Incompatibility::RootUuid,
    )?;
    require(
        root.format_profile == erofs::EROFS_FORMAT_PROFILE
            && root.formatter_revision == erofs::EROFS_UTILS_REVISION,
        Incompatibility::RootFormat,
    )?;
    nonzero(&root.formatter_digest)?;
    if let Some(builder) = root.builder_image_digest.as_ref() {
        nonzero(builder)?;
    }
    let size = root.descriptor.size;
    require(
        size > 0 && size <= profile.max_root_bytes && size.is_multiple_of(EROFS_BLOCK_BYTES),
        Incompatibility::RootSize,
    )
}

fn require_overlay(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    let overlay = &manifest.overlay;
    require(
        overlay.uuid_derivation_version == OVERLAY_UUID_DERIVATION_VERSION
            && overlay.feature_profile == overlay_feature_profile(),
        Incompatibility::OverlayProfile,
    )?;
    let (Some(first), Some(last)) = (overlay.templates.first(), overlay.templates.last()) else {
        return Err(CompileError::incompatible(Incompatibility::OverlayCapacity));
    };
    for template in &overlay.templates {
        require(
            template.capacity >= MINIMUM_OVERLAY_BYTES
                && template.capacity.is_multiple_of(OVERLAY_ALIGNMENT_BYTES)
                && profile.overlay_capacities.contains(&template.capacity),
            Incompatibility::OverlayCapacity,
        )?;
        require(
            template.descriptor.size == template.capacity,
            Incompatibility::OverlaySize,
        )?;
    }
    require(
        overlay.minimum_capacity == first.capacity
            && overlay.maximum_capacity == last.capacity
            && overlay.minimum_capacity <= overlay.maximum_capacity,
        Incompatibility::OverlayBounds,
    )
}

fn require_machine_artifacts(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    let kernel = &manifest.kernel;
    require(
        kernel.elf_pvh_contract_version == ELF_PVH_CONTRACT_VERSION
            && kernel.cpu_architecture == "x86_64",
        Incompatibility::KernelContract,
    )?;
    nonzero(&kernel.config_digest)?;
    bounded(
        kernel.descriptor.size,
        profile.max_kernel_bytes,
        Incompatibility::KernelSize,
    )?;
    require(
        manifest.initramfs.layout_version == INITRAMFS_LAYOUT_VERSION,
        Incompatibility::InitramfsLayout,
    )?;
    nonzero(&manifest.initramfs.early_init_digest)?;
    bounded(
        manifest.initramfs.descriptor.size,
        profile.max_initramfs_bytes,
        Incompatibility::InitramfsSize,
    )?;
    let agent = &manifest.guest_agent;
    bounded(
        agent.descriptor.size,
        profile.max_executable_bytes,
        Incompatibility::GuestAgentSize,
    )?;
    require(
        !agent.build_provenance.is_empty() && agent.build_provenance.len() <= 256,
        Incompatibility::GuestAgentProvenance,
    )?;
    require(
        agent.application_protocol_version == profile.application_protocol_version
            && agent.handshake_protocol_version == profile.handshake_protocol_version,
        Incompatibility::GuestProtocol,
    )
}

/// Sums every declared artifact size with checked arithmetic.
///
/// A manifest whose sizes cannot be summed is rejected before any allocation or seek uses them.
fn require_total_size(manifest: &GenerationManifest) -> Result<(), CompileError> {
    let mut total = 0_u64;
    for descriptor in manifest.descriptors() {
        total = total
            .checked_add(descriptor.size)
            .ok_or_else(|| CompileError::incompatible(Incompatibility::ArtifactSizeOverflow))?;
    }
    Ok(())
}

pub(super) fn require(condition: bool, reason: Incompatibility) -> Result<(), CompileError> {
    if condition {
        return Ok(());
    }
    Err(CompileError::incompatible(reason))
}

pub(super) fn nonzero(digest: &Sha256Digest) -> Result<(), CompileError> {
    require(
        digest.as_bytes().iter().any(|byte| *byte != 0),
        Incompatibility::ZeroDigest,
    )
}

pub(super) fn bounded(
    size: u64,
    maximum: u64,
    reason: Incompatibility,
) -> Result<(), CompileError> {
    require(size > 0 && size <= maximum, reason)
}

pub(super) fn descriptor_nonzero(
    descriptor: &ArtifactDescriptor,
    reason: Incompatibility,
) -> Result<(), CompileError> {
    nonzero(&descriptor.digest)?;
    require(descriptor.size > 0, reason)
}
