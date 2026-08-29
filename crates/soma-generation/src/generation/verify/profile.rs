//! Host-profile compatibility of one decoded manifest.
//!
//! Every field arrives from bytes a hostile party may have produced, so each is validated
//! before the compiler acts on it.

use crate::generation::{
    contracts,
    erofs::{self, derive_root_uuid},
    error::{CompileError, CompileErrorKind, CompilePhase},
    initramfs::INITRAMFS_LAYOUT_VERSION,
    kernel::ELF_PVH_CONTRACT_VERSION,
    manifest::GenerationManifest,
    overlay::OVERLAY_UUID_DERIVATION_VERSION,
    request::CompilerProfile,
};

pub(super) fn require_profile(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    let root_uuid = derive_root_uuid(&manifest.tree.digest);
    let expected_features = crate::generation::overlay::overlay_feature_profile();
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
        || !manifest
            .overlay
            .templates
            .iter()
            .any(|template| template.capacity == manifest.template.writable_storage_bytes)
    {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            CompileErrorKind::Unsupported,
        ));
    }
    Ok(())
}
