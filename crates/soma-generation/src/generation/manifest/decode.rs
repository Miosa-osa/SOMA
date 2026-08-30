use soma::OciPlatform;

use super::{
    CANDIDATE_MAGIC, GenerationManifest, GuestAgentBinding, InitramfsBinding, KernelBinding, MAGIC,
    MANIFEST_SCHEMA_VERSION, MAX_COMMAND_LINE, MAX_MANIFEST_BYTES, MAX_SHORT_STRING, MAX_TEMPLATES,
    MachineShapeBinding, OverlayBinding, OverlayTemplate, RepairBinding, RootBinding,
    SnapshotBinding, SourceBinding, TemplateBinding, TreeBinding,
};
use crate::generation::{
    artifacts::{ArtifactRole, Sha256Digest},
    error::{CompileError, CompileErrorKind, CompilePhase},
    template::{MAX_WORKLOAD_PROBE_BYTES, NetworkPolicyClass},
};

/// Decodes canonical `SOMAGEN` v2 bytes while treating every field as hostile.
///
/// # Errors
///
/// Returns [`CompileErrorKind::InvalidInput`] for a bad magic, unknown group tag, wrong
/// descriptor role, unsupported media type, duplicate descriptor, non-ascending template,
/// unsupported platform, trailing bytes, or truncation, [`CompileErrorKind::Unsupported`] for
/// another schema version, and [`CompileErrorKind::LimitExceeded`] for oversized fields.
pub fn decode_manifest(bytes: &[u8]) -> Result<GenerationManifest, CompileError> {
    decode_with(bytes, *MAGIC)
}

/// Decodes canonical `SOMACAN` Candidate bytes; a ready manifest is rejected here.
///
/// # Errors
///
/// Returns the same classifications as [`decode_manifest`].
pub fn decode_candidate(bytes: &[u8]) -> Result<GenerationManifest, CompileError> {
    decode_with(bytes, *CANDIDATE_MAGIC)
}

fn decode_with(bytes: &[u8], magic: [u8; 8]) -> Result<GenerationManifest, CompileError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(limit());
    }
    let mut decoder = Decoder {
        bytes,
        offset: 0,
        seen: Vec::new(),
    };
    if decoder.consume(8)? != magic {
        return Err(invalid());
    }
    if decoder.u16()? != MANIFEST_SCHEMA_VERSION {
        return Err(CompileError::new(
            CompilePhase::EncodeManifest,
            CompileErrorKind::Unsupported,
        ));
    }
    let compiler_policy_version = decoder.u16()?;
    let source = decoder.source()?;
    decoder.tag(3)?;
    let tree = TreeBinding {
        digest: decoder.digest()?,
        size: decoder.u64()?,
    };
    let root = decoder.root()?;
    let overlay = decoder.overlay()?;
    decoder.tag(6)?;
    let kernel = KernelBinding {
        descriptor: decoder.descriptor(ArtifactRole::Kernel)?,
        elf_pvh_contract_version: decoder.u16()?,
        config_digest: decoder.digest()?,
        cpu_architecture: decoder.short_string()?,
    };
    decoder.tag(7)?;
    let initramfs = InitramfsBinding {
        descriptor: decoder.descriptor(ArtifactRole::Initramfs)?,
        layout_version: decoder.u16()?,
        early_init_digest: decoder.digest()?,
    };
    decoder.tag(8)?;
    let guest_agent = GuestAgentBinding {
        descriptor: decoder.descriptor(ArtifactRole::GuestAgent)?,
        build_provenance: decoder.short_string()?,
        application_protocol_version: decoder.u16()?,
        handshake_protocol_version: decoder.u16()?,
    };
    let command_line = decoder.command_line()?;
    let machine_contract = decoder.contract(10)?;
    let device_contract = decoder.contract(11)?;
    let cpu_template = decoder.contract(12)?;
    decoder.tag(13)?;
    let shape = MachineShapeBinding {
        memory_bytes: decoder.u64()?,
        vcpu_count: decoder.u16()?,
        memory_slot_layout_version: decoder.u16()?,
        launch_page_layout_version: decoder.u16()?,
    };
    let snapshot = decoder.snapshot()?;
    decoder.tag(15)?;
    let repair = RepairBinding {
        policy_version: decoder.u16()?,
        readiness_command_digest: decoder.digest()?,
    };
    let template = decoder.template()?;
    if decoder.offset != bytes.len() {
        return Err(invalid());
    }
    Ok(GenerationManifest {
        compiler_policy_version,
        source,
        tree,
        root,
        overlay,
        kernel,
        initramfs,
        guest_agent,
        command_line,
        machine_contract,
        device_contract,
        cpu_template,
        shape,
        snapshot,
        repair,
        template,
    })
}

mod primitives;

pub(super) struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    seen: Vec<Sha256Digest>,
}

impl Decoder<'_> {
    fn source(&mut self) -> Result<SourceBinding, CompileError> {
        self.tag(2)?;
        let oci_manifest_digest = self.digest()?;
        let operating_system = self.short_string()?;
        let architecture = self.short_string()?;
        let variant = self.optional_string()?;
        if operating_system != "linux" || architecture != "amd64" {
            return Err(invalid());
        }
        let platform =
            OciPlatform::new(operating_system, architecture, variant).map_err(|_| invalid())?;
        Ok(SourceBinding {
            oci_manifest_digest,
            platform,
        })
    }

    fn root(&mut self) -> Result<RootBinding, CompileError> {
        self.tag(4)?;
        Ok(RootBinding {
            descriptor: self.descriptor(ArtifactRole::ErofsRoot)?,
            uuid: self.array()?,
            format_profile: self.short_string()?,
            formatter_digest: self.digest()?,
            formatter_revision: self.short_string()?,
            builder_environment_digest: self.digest()?,
        })
    }

    fn overlay(&mut self) -> Result<OverlayBinding, CompileError> {
        self.tag(5)?;
        let uuid_derivation_version = self.u16()?;
        let feature_profile = self.short_string()?;
        let minimum_capacity = self.u64()?;
        let maximum_capacity = self.u64()?;
        let count = usize::from(self.u16()?);
        if count > MAX_TEMPLATES {
            return Err(limit());
        }
        let mut templates: Vec<OverlayTemplate> = Vec::with_capacity(count);
        for _ in 0..count {
            let template = OverlayTemplate {
                capacity: self.u64()?,
                descriptor: self.descriptor(ArtifactRole::OverlayTemplate)?,
            };
            if templates
                .last()
                .is_some_and(|previous| template.capacity <= previous.capacity)
            {
                return Err(invalid());
            }
            templates.push(template);
        }
        Ok(OverlayBinding {
            uuid_derivation_version,
            feature_profile,
            minimum_capacity,
            maximum_capacity,
            templates,
        })
    }

    fn command_line(&mut self) -> Result<Vec<u8>, CompileError> {
        self.tag(9)?;
        let length = usize::from(self.u16()?);
        if length > MAX_COMMAND_LINE {
            return Err(limit());
        }
        let command_line = self.consume(length)?.to_vec();
        if command_line.contains(&0) {
            return Err(invalid());
        }
        Ok(command_line)
    }

    fn snapshot(&mut self) -> Result<SnapshotBinding, CompileError> {
        self.tag(14)?;
        match self.u8()? {
            0 => Ok(SnapshotBinding::Absent),
            1 => Ok(SnapshotBinding::Captured {
                format_version: self.u16()?,
                memory: self.descriptor(ArtifactRole::MemorySnapshot)?,
                overlay: self.descriptor(ArtifactRole::OverlaySnapshot)?,
                state: self.descriptor(ArtifactRole::StateManifest)?,
                capture_point_version: self.u16()?,
            }),
            _ => Err(invalid()),
        }
    }

    fn template(&mut self) -> Result<TemplateBinding, CompileError> {
        self.tag(16)?;
        let writable_storage_bytes = self.u64()?;
        let network_policy_class = NetworkPolicyClass::from_code(self.u8()?).ok_or_else(invalid)?;
        let network_policy_digest = self.digest()?;
        let workload_probe = match self.u8()? {
            0 => None,
            1 => {
                let length = usize::from(self.u16()?);
                if length == 0 || length > MAX_WORKLOAD_PROBE_BYTES {
                    return Err(limit());
                }
                let probe = self.consume(length)?.to_vec();
                if probe.contains(&0) {
                    return Err(invalid());
                }
                Some(probe)
            }
            _ => return Err(invalid()),
        };
        Ok(TemplateBinding {
            writable_storage_bytes,
            network_policy_class,
            network_policy_digest,
            workload_probe,
            ttl_seconds: self.u64()?,
        })
    }
}

pub(super) const fn invalid() -> CompileError {
    CompileError::new(CompilePhase::EncodeManifest, CompileErrorKind::InvalidInput)
}

pub(super) const fn limit() -> CompileError {
    CompileError::new(
        CompilePhase::EncodeManifest,
        CompileErrorKind::LimitExceeded,
    )
}
