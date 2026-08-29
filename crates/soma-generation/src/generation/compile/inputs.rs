use std::{fs::File, io::Read as _, path::Path};

use crate::generation::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
    initramfs::{InitramfsContents, build_initramfs, verify_initramfs},
    kernel::{VerifiedKernel, verify_kernel},
    kernel_config::{MAX_CONFIG_BYTES, VerifiedKernelConfig, verify_kernel_config},
    request::{CompileGeneration, CompilerProfile},
};
use crate::{ImportPhase, normalize::TREE_MEDIA_TYPE, oci::Descriptor, store::Store};

/// The bounded, verified machine inputs read before any formatter runs.
pub(super) struct MachineArtifacts {
    pub(super) kernel_bytes: Vec<u8>,
    pub(super) kernel: VerifiedKernel,
    pub(super) config: VerifiedKernelConfig,
    pub(super) initramfs: Vec<u8>,
    pub(super) contents: InitramfsContents,
    pub(super) guest_agent: Vec<u8>,
}

impl MachineArtifacts {
    pub(super) fn build(
        request: CompileGeneration<'_>,
        profile: &CompilerProfile,
    ) -> Result<Self, CompileError> {
        let inputs = request.inputs;
        let kernel_bytes = read_bounded(
            inputs.kernel,
            profile.max_kernel_bytes,
            CompilePhase::VerifyKernel,
        )?;
        let kernel = verify_kernel(&kernel_bytes)?;
        let config_limit = u64::try_from(MAX_CONFIG_BYTES).unwrap_or(u64::MAX);
        let config = verify_kernel_config(&read_bounded(
            inputs.kernel_config,
            config_limit,
            CompilePhase::VerifyKernel,
        )?)?;
        let early_init = read_bounded(
            inputs.early_init,
            profile.max_executable_bytes,
            CompilePhase::BuildInitramfs,
        )?;
        let guest_agent = read_bounded(
            inputs.guest_agent,
            profile.max_executable_bytes,
            CompilePhase::BuildInitramfs,
        )?;
        let initramfs = build_initramfs(&early_init, &guest_agent, profile.max_initramfs_bytes)?;
        let contents = verify_initramfs(&initramfs)?;
        if contents.guest_agent_digest != Sha256Digest::of(&guest_agent) {
            return Err(CompileError::new(
                CompilePhase::VerifyInitramfs,
                CompileErrorKind::Integrity,
            ));
        }
        Ok(Self {
            kernel_bytes,
            kernel,
            config,
            initramfs,
            contents,
            guest_agent,
        })
    }
}

pub(super) fn read_tree(
    store: &Store,
    digest: &Sha256Digest,
    size: u64,
) -> Result<Vec<u8>, CompileError> {
    let descriptor = Descriptor {
        media_type: TREE_MEDIA_TYPE.to_owned(),
        digest: digest.to_oci(),
        size,
        platform: None,
    };
    let mut file = store
        .open_verified_blob(
            &descriptor,
            super::MAX_TREE_MANIFEST_BYTES,
            ImportPhase::Publish,
        )
        .map_err(|error| CompileError::from_import(CompilePhase::ResolveInputs, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| CompileError::new(CompilePhase::ResolveInputs, CompileErrorKind::Io))?;
    Ok(bytes)
}

pub(super) fn read_bounded(
    path: &Path,
    maximum: u64,
    phase: CompilePhase,
) -> Result<Vec<u8>, CompileError> {
    let file = File::open(path).map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?;
    let mut bytes = Vec::new();
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?;
    let length = u64::try_from(bytes.len())
        .map_err(|_| CompileError::new(phase, CompileErrorKind::LimitExceeded))?;
    if length > maximum {
        return Err(CompileError::new(phase, CompileErrorKind::LimitExceeded));
    }
    Ok(bytes)
}
