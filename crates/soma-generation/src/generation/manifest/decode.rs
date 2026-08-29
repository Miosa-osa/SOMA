use soma::OciPlatform;

use super::{
    GenerationManifest, GuestAgentBinding, InitramfsBinding, KernelBinding, MAGIC,
    MANIFEST_SCHEMA_VERSION, MAX_COMMAND_LINE, MAX_MANIFEST_BYTES, MAX_SHORT_STRING, MAX_TEMPLATES,
    MachineShape, OverlayBinding, OverlayTemplate, RepairBinding, RootBinding, SnapshotBinding,
    SourceBinding, TreeBinding,
};
use crate::generation::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    contracts::ContractBinding,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

/// Decodes canonical `SOMAGEN` v1 bytes while treating every field as hostile.
///
/// # Errors
///
/// Returns [`CompileErrorKind::InvalidInput`] for a bad magic, unknown group tag, wrong
/// descriptor role, unsupported media type, duplicate descriptor, non-ascending template,
/// unsupported platform, trailing bytes, or truncation, [`CompileErrorKind::Unsupported`] for
/// another schema version, and [`CompileErrorKind::LimitExceeded`] for oversized fields.
pub fn decode_manifest(bytes: &[u8]) -> Result<GenerationManifest, CompileError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(limit());
    }
    let mut decoder = Decoder {
        bytes,
        offset: 0,
        seen: Vec::new(),
    };
    if decoder.consume(8)? != MAGIC {
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
    let shape = MachineShape {
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
    })
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    seen: Vec<Sha256Digest>,
}

impl<'a> Decoder<'a> {
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
            builder_image_digest: self.optional_digest()?,
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
                state: self.descriptor(ArtifactRole::StateManifest)?,
                capture_point_version: self.u16()?,
            }),
            _ => Err(invalid()),
        }
    }

    fn tag(&mut self, expected: u8) -> Result<(), CompileError> {
        if self.u8()? != expected {
            return Err(invalid());
        }
        Ok(())
    }

    fn contract(&mut self, tag: u8) -> Result<ContractBinding, CompileError> {
        self.tag(tag)?;
        Ok(ContractBinding {
            version: self.u16()?,
            digest: self.digest()?,
        })
    }

    fn descriptor(&mut self, expected: ArtifactRole) -> Result<ArtifactDescriptor, CompileError> {
        let role = ArtifactRole::from_code(self.u8()?).ok_or_else(invalid)?;
        let media_type = self.short_string()?;
        if role != expected || media_type != role.media_type() {
            return Err(invalid());
        }
        let digest = self.digest()?;
        let size = self.u64()?;
        if self.seen.contains(&digest) {
            return Err(invalid());
        }
        self.seen.push(digest);
        Ok(ArtifactDescriptor { role, digest, size })
    }

    fn optional_digest(&mut self) -> Result<Option<Sha256Digest>, CompileError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.digest()?)),
            _ => Err(invalid()),
        }
    }

    fn optional_string(&mut self) -> Result<Option<String>, CompileError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.short_string()?)),
            _ => Err(invalid()),
        }
    }

    fn short_string(&mut self) -> Result<String, CompileError> {
        let length = usize::from(self.u16()?);
        if length > MAX_SHORT_STRING {
            return Err(limit());
        }
        let value = self.consume(length)?;
        if value.contains(&0) {
            return Err(invalid());
        }
        String::from_utf8(value.to_vec()).map_err(|_| invalid())
    }

    fn digest(&mut self) -> Result<Sha256Digest, CompileError> {
        Ok(Sha256Digest::from_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CompileError> {
        self.consume(N)?.try_into().map_err(|_| invalid())
    }

    fn consume(&mut self, count: usize) -> Result<&'a [u8], CompileError> {
        let end = self.offset.checked_add(count).ok_or_else(invalid)?;
        let value = self.bytes.get(self.offset..end).ok_or_else(invalid)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, CompileError> {
        Ok(self.consume(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, CompileError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, CompileError> {
        Ok(u64::from_be_bytes(self.array()?))
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
