use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use soma::{GenerationId, WorkloadIdentity};

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    contracts,
    erofs::{self, ErofsEvidence, derive_root_uuid},
    error::{CompileError, CompileErrorKind, CompilePhase},
    initramfs::INITRAMFS_LAYOUT_VERSION,
    kernel::{ELF_PVH_CONTRACT_VERSION, VerifiedKernel},
    manifest::{
        GenerationManifest, GuestAgentBinding, InitramfsBinding, KernelBinding, MachineShape,
        OverlayBinding, OverlayTemplate, RepairBinding, RootBinding, SnapshotBinding,
        SourceBinding, TreeBinding,
    },
    overlay::{self, OverlayEvidence},
    publish::{PublishedGeneration, publish_manifest},
    request::CompileGeneration,
};

mod inputs;

use crate::{ImportPhase, store::Store};
use inputs::{MachineArtifacts, read_tree};

const MAX_TREE_MANIFEST_BYTES: u64 = 512 * 1024 * 1024;
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Typed state for the compiler phases that have no implementation yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnimplementedPhase {
    /// Guest boot, authenticated repair, quiesce, and snapshot capture (phase 4).
    BootAndCapture,
    /// Host-profile certification (phase 5).
    Certification,
}

/// The outcome of one Generation compilation without boot, capture, or certification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledGeneration {
    /// The published manifest and its identity.
    pub published: PublishedGeneration,
    /// The verified kernel facts.
    pub kernel: VerifiedKernel,
    /// EROFS build evidence.
    pub erofs: ErofsEvidence,
    /// Overlay-template build evidence.
    pub overlay: OverlayEvidence,
    /// The phases this compiler cannot perform; the Generation is not launchable until they run.
    pub unimplemented: [UnimplementedPhase; 2],
}

impl CompiledGeneration {
    /// Returns the identity derived from the published manifest bytes.
    #[must_use]
    pub const fn id(&self) -> &GenerationId {
        &self.published.id
    }
}

/// Compiles immutable machine artifacts from one verified normalized tree and publishes the
/// canonical manifest last.
///
/// # Errors
///
/// Returns a redacted [`CompileError`] naming the phase and classification of the first failure.
/// Failure leaves no discoverable Generation manifest.
pub fn compile_generation(
    request: CompileGeneration<'_>,
) -> Result<CompiledGeneration, CompileError> {
    let profile = request.profile;
    profile.validate()?;
    let workload = request.normalized.workload();
    require_x86_64_workload(workload)?;
    let store = Store::open(request.store)
        .map_err(|error| CompileError::from_import(CompilePhase::ResolveInputs, error))?;
    let tree_digest = Sha256Digest::from_oci(request.normalized.tree_manifest_digest());
    let tree_size = request.normalized.tree_manifest_size();
    let tree_bytes = read_tree(&store, &tree_digest, tree_size)?;
    let machine = MachineArtifacts::build(request, profile)?;

    let staging = Staging::create(request.staging)?;
    let (root_descriptor, erofs_evidence) = erofs::compile_root(
        request.toolchain.erofs_utils,
        profile,
        &tree_bytes,
        &tree_digest,
        &store,
        &staging.path,
    )?;
    let (templates, overlay_evidence) = overlay::compile_overlay_templates(
        request.toolchain.e2fsprogs,
        profile,
        &store,
        &staging.path,
    )?;
    drop(staging);

    let kernel_descriptor = store_bytes(&store, &machine.kernel_bytes, ArtifactRole::Kernel)?;
    let initramfs_descriptor = store_bytes(&store, &machine.initramfs, ArtifactRole::Initramfs)?;
    let agent_descriptor = store_bytes(&store, &machine.guest_agent, ArtifactRole::GuestAgent)?;

    let manifest = GenerationManifest {
        compiler_policy_version: profile.policy_version,
        source: SourceBinding {
            oci_manifest_digest: Sha256Digest::from_oci(workload.manifest_digest()),
            platform: workload.platform().clone(),
        },
        tree: TreeBinding {
            digest: tree_digest,
            size: tree_size,
        },
        root: RootBinding {
            descriptor: root_descriptor,
            uuid: derive_root_uuid(&tree_digest),
            format_profile: erofs::EROFS_FORMAT_PROFILE.to_owned(),
            formatter_digest: erofs_evidence.formatter_digest,
            formatter_revision: erofs_evidence.formatter_revision.clone(),
            builder_image_digest: None,
        },
        overlay: overlay_binding(templates),
        kernel: KernelBinding {
            descriptor: kernel_descriptor,
            elf_pvh_contract_version: ELF_PVH_CONTRACT_VERSION,
            config_digest: machine.config.digest,
            cpu_architecture: "x86_64".to_owned(),
        },
        initramfs: InitramfsBinding {
            descriptor: initramfs_descriptor,
            layout_version: INITRAMFS_LAYOUT_VERSION,
            early_init_digest: machine.contents.early_init_digest,
        },
        guest_agent: GuestAgentBinding {
            descriptor: agent_descriptor,
            build_provenance: profile.guest_agent_provenance.clone(),
            application_protocol_version: profile.application_protocol_version,
            handshake_protocol_version: profile.handshake_protocol_version,
        },
        command_line: contracts::kernel_command_line_v1(),
        machine_contract: contracts::machine_contract_v1(),
        device_contract: contracts::device_contract_v1(),
        cpu_template: contracts::cpu_template_v1(),
        shape: MachineShape {
            memory_bytes: profile.memory_bytes,
            vcpu_count: profile.vcpu_count,
            memory_slot_layout_version: 1,
            launch_page_layout_version: 1,
        },
        snapshot: SnapshotBinding::Absent,
        repair: RepairBinding {
            policy_version: 1,
            readiness_command_digest: contracts::readiness_command_digest(),
        },
    };
    let published = publish_manifest(&store, &manifest)?;
    Ok(CompiledGeneration {
        published,
        kernel: machine.kernel,
        erofs: erofs_evidence,
        overlay: overlay_evidence,
        unimplemented: [
            UnimplementedPhase::BootAndCapture,
            UnimplementedPhase::Certification,
        ],
    })
}

fn require_x86_64_workload(workload: &WorkloadIdentity) -> Result<(), CompileError> {
    let platform = workload.platform();
    if platform.operating_system() != "linux"
        || platform.architecture() != "amd64"
        || platform.variant().is_some()
        || workload.generation_id().is_some()
    {
        return Err(CompileError::new(
            CompilePhase::ResolveInputs,
            CompileErrorKind::Unsupported,
        ));
    }
    Ok(())
}

fn overlay_binding(templates: Vec<OverlayTemplate>) -> OverlayBinding {
    OverlayBinding {
        uuid_derivation_version: overlay::OVERLAY_UUID_DERIVATION_VERSION,
        feature_profile: overlay::overlay_feature_profile(),
        minimum_capacity: templates.first().map_or(0, |template| template.capacity),
        maximum_capacity: templates.last().map_or(0, |template| template.capacity),
        templates,
    }
}

fn store_bytes(
    store: &Store,
    bytes: &[u8],
    role: ArtifactRole,
) -> Result<ArtifactDescriptor, CompileError> {
    let stored = store
        .put_bytes(bytes, role.media_type(), ImportPhase::Publish)
        .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    Ok(ArtifactDescriptor {
        role,
        digest: Sha256Digest::from_oci(&stored.digest),
        size: stored.size,
    })
}

struct Staging {
    path: PathBuf,
}

impl Staging {
    fn create(parent: &Path) -> Result<Self, CompileError> {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("soma-generation-{}-{sequence}", std::process::id()));
        fs::create_dir(&path)
            .map_err(|_| CompileError::new(CompilePhase::ResolveInputs, CompileErrorKind::Io))?;
        Ok(Self { path })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
