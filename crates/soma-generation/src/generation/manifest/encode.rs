use super::{
    CANDIDATE_MAGIC, GenerationManifest, MAGIC, MANIFEST_SCHEMA_VERSION, MAX_COMMAND_LINE,
    MAX_MANIFEST_BYTES, MAX_SHORT_STRING, MAX_TEMPLATES, OverlayBinding, RootBinding,
    SnapshotBinding, TemplateBinding,
};
use crate::generation::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    contracts::ContractBinding,
    error::{CompileError, CompileErrorKind, CompilePhase},
    template::MAX_WORKLOAD_PROBE_BYTES,
};

/// Encodes the canonical `SOMAGEN` v1 bytes whose SHA-256 is the `GenerationId`.
///
/// Fields are emitted in fixed order with big-endian integers, explicit presence bytes, and
/// length-prefixed bounded byte strings; no map or implementation-dependent ordering exists.
///
/// # Errors
///
/// Returns [`CompileErrorKind::LimitExceeded`] when a string, list, or the whole manifest
/// exceeds its bound, and [`CompileErrorKind::InvalidInput`] for an unsupported platform, a
/// descriptor with the wrong role, or overlay templates that are not strictly ascending.
pub fn encode_manifest(manifest: &GenerationManifest) -> Result<Vec<u8>, CompileError> {
    encode_with(manifest, *MAGIC)
}

/// Encodes the same canonical fields under the Candidate magic.
///
/// The bytes are deliberately not decodable as a ready Generation manifest, so a Candidate can
/// never be resolved for Launch even when its digest is known.
///
/// # Errors
///
/// Returns the same classifications as [`encode_manifest`].
pub fn encode_candidate(manifest: &GenerationManifest) -> Result<Vec<u8>, CompileError> {
    encode_with(manifest, *CANDIDATE_MAGIC)
}

fn encode_with(manifest: &GenerationManifest, magic: [u8; 8]) -> Result<Vec<u8>, CompileError> {
    let mut encoder = Encoder { bytes: Vec::new() };
    encoder.bytes(&magic)?;
    encoder.u16(MANIFEST_SCHEMA_VERSION)?;
    encoder.u16(manifest.compiler_policy_version)?;

    encoder.u8(2)?;
    encoder.digest(&manifest.source.oci_manifest_digest)?;
    let platform = &manifest.source.platform;
    if platform.operating_system() != "linux" || platform.architecture() != "amd64" {
        return Err(invalid());
    }
    encoder.short_string(platform.operating_system().as_bytes())?;
    encoder.short_string(platform.architecture().as_bytes())?;
    encoder.optional_string(platform.variant().map(str::as_bytes))?;

    encoder.u8(3)?;
    encoder.digest(&manifest.tree.digest)?;
    encoder.u64(manifest.tree.size)?;
    encoder.root(&manifest.root)?;
    encoder.overlay(&manifest.overlay)?;

    encoder.u8(6)?;
    encoder.descriptor(&manifest.kernel.descriptor, ArtifactRole::Kernel)?;
    encoder.u16(manifest.kernel.elf_pvh_contract_version)?;
    encoder.digest(&manifest.kernel.config_digest)?;
    encoder.short_string(manifest.kernel.cpu_architecture.as_bytes())?;

    encoder.u8(7)?;
    encoder.descriptor(&manifest.initramfs.descriptor, ArtifactRole::Initramfs)?;
    encoder.u16(manifest.initramfs.layout_version)?;
    encoder.digest(&manifest.initramfs.early_init_digest)?;

    encoder.u8(8)?;
    let agent = &manifest.guest_agent;
    encoder.descriptor(&agent.descriptor, ArtifactRole::GuestAgent)?;
    encoder.short_string(agent.build_provenance.as_bytes())?;
    encoder.u16(agent.application_protocol_version)?;
    encoder.u16(agent.handshake_protocol_version)?;

    encoder.u8(9)?;
    if manifest.command_line.len() > MAX_COMMAND_LINE || manifest.command_line.contains(&0) {
        return Err(limit());
    }
    encoder.u16(u16::try_from(manifest.command_line.len()).map_err(|_| limit())?)?;
    encoder.bytes(&manifest.command_line)?;

    encoder.contract(10, &manifest.machine_contract)?;
    encoder.contract(11, &manifest.device_contract)?;
    encoder.contract(12, &manifest.cpu_template)?;

    encoder.u8(13)?;
    encoder.u64(manifest.shape.memory_bytes)?;
    encoder.u16(manifest.shape.vcpu_count)?;
    encoder.u16(manifest.shape.memory_slot_layout_version)?;
    encoder.u16(manifest.shape.launch_page_layout_version)?;

    encoder.u8(14)?;
    match manifest.snapshot {
        SnapshotBinding::Absent => encoder.u8(0)?,
        SnapshotBinding::Captured {
            format_version,
            memory,
            state,
            capture_point_version,
        } => {
            encoder.u8(1)?;
            encoder.u16(format_version)?;
            encoder.descriptor(&memory, ArtifactRole::MemorySnapshot)?;
            encoder.descriptor(&state, ArtifactRole::StateManifest)?;
            encoder.u16(capture_point_version)?;
        }
    }

    encoder.u8(15)?;
    encoder.u16(manifest.repair.policy_version)?;
    encoder.digest(&manifest.repair.readiness_command_digest)?;
    encoder.template(&manifest.template)?;
    Ok(encoder.bytes)
}

struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn root(&mut self, root: &RootBinding) -> Result<(), CompileError> {
        self.u8(4)?;
        self.descriptor(&root.descriptor, ArtifactRole::ErofsRoot)?;
        self.bytes(&root.uuid)?;
        self.short_string(root.format_profile.as_bytes())?;
        self.digest(&root.formatter_digest)?;
        self.short_string(root.formatter_revision.as_bytes())?;
        self.digest(&root.builder_environment_digest)
    }

    fn overlay(&mut self, overlay: &OverlayBinding) -> Result<(), CompileError> {
        self.u8(5)?;
        self.u16(overlay.uuid_derivation_version)?;
        self.short_string(overlay.feature_profile.as_bytes())?;
        self.u64(overlay.minimum_capacity)?;
        self.u64(overlay.maximum_capacity)?;
        if overlay.templates.len() > MAX_TEMPLATES
            || overlay
                .templates
                .windows(2)
                .any(|pair| pair[1].capacity <= pair[0].capacity)
        {
            return Err(invalid());
        }
        self.u16(u16::try_from(overlay.templates.len()).map_err(|_| limit())?)?;
        for template in &overlay.templates {
            self.u64(template.capacity)?;
            self.descriptor(&template.descriptor, ArtifactRole::OverlayTemplate)?;
        }
        Ok(())
    }

    fn template(&mut self, template: &TemplateBinding) -> Result<(), CompileError> {
        self.u8(16)?;
        self.u64(template.writable_storage_bytes)?;
        self.u8(template.network_policy_class.code())?;
        self.digest(&template.network_policy_digest)?;
        match &template.workload_probe {
            Some(probe) => {
                if probe.is_empty() || probe.len() > MAX_WORKLOAD_PROBE_BYTES || probe.contains(&0)
                {
                    return Err(invalid());
                }
                self.u8(1)?;
                self.u16(u16::try_from(probe.len()).map_err(|_| limit())?)?;
                self.bytes(probe)?;
            }
            None => self.u8(0)?,
        }
        self.u64(template.ttl_seconds)
    }

    fn contract(&mut self, tag: u8, binding: &ContractBinding) -> Result<(), CompileError> {
        self.u8(tag)?;
        self.u16(binding.version)?;
        self.digest(&binding.digest)
    }

    fn descriptor(
        &mut self,
        descriptor: &ArtifactDescriptor,
        expected: ArtifactRole,
    ) -> Result<(), CompileError> {
        if descriptor.role != expected {
            return Err(invalid());
        }
        self.u8(descriptor.role.code())?;
        self.short_string(descriptor.media_type().as_bytes())?;
        self.digest(&descriptor.digest)?;
        self.u64(descriptor.size)
    }

    fn optional_string(&mut self, value: Option<&[u8]>) -> Result<(), CompileError> {
        match value {
            Some(value) => {
                self.u8(1)?;
                self.short_string(value)
            }
            None => self.u8(0),
        }
    }

    fn short_string(&mut self, value: &[u8]) -> Result<(), CompileError> {
        if value.len() > MAX_SHORT_STRING || value.contains(&0) {
            return Err(limit());
        }
        self.u16(u16::try_from(value.len()).map_err(|_| limit())?)?;
        self.bytes(value)
    }

    fn digest(&mut self, digest: &Sha256Digest) -> Result<(), CompileError> {
        self.bytes(digest.as_bytes())
    }

    fn u8(&mut self, value: u8) -> Result<(), CompileError> {
        self.bytes(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<(), CompileError> {
        self.bytes(&value.to_be_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<(), CompileError> {
        self.bytes(&value.to_be_bytes())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), CompileError> {
        if self
            .bytes
            .len()
            .checked_add(value.len())
            .ok_or_else(limit)?
            > MAX_MANIFEST_BYTES
        {
            return Err(limit());
        }
        self.bytes.extend_from_slice(value);
        Ok(())
    }
}

const fn invalid() -> CompileError {
    CompileError::new(CompilePhase::EncodeManifest, CompileErrorKind::InvalidInput)
}

const fn limit() -> CompileError {
    CompileError::new(
        CompilePhase::EncodeManifest,
        CompileErrorKind::LimitExceeded,
    )
}
