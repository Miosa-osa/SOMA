//! Live capture of a running sandbox at the disconnected repair point.
//!
//! The machine is captured before any launch page is written, so the memory image contains no
//! Instance identity, no session, no key, and no network authority: there is nothing to scrub
//! and nothing that could be replayed into a second Instance. The quiesce proofs, the read
//! order, and the publication order are all driven through the codec's typed schedules, so a
//! step performed out of order is a typed failure rather than a silently unsafe snapshot.

use std::{fs::File, time::Instant};

use super::{
    artifacts::{self, SnapshotPaths, Staging},
    device,
    error::{Artifact, SnapshotError},
    marker, platform, profile, quiesce, vcpu,
};
use crate::snapshot::{
    Digest,
    capture::{CaptureStep, Quiesce, QuiescePrecondition},
    device_state::DeviceState,
    kvm_state::{MemorySlot, VmState},
    manifest::{Architecture, CandidateId, Manifest, ManifestHeader, PageSize},
    memory::MemoryDescriptor,
    section::{Section, SectionRole},
};
use crate::virtio::{GuestAddress, GuestMemory as _, Slot};
use crate::x86_64::{layout, sandbox::SandboxMachine};

/// Bytes copied out of guest RAM per pass.
const CHUNK: usize = 1 << 20;

/// Everything a capture needs beyond the running machine itself.
pub struct CaptureRequest<'a> {
    /// Where the three objects are published.
    pub paths: SnapshotPaths,
    /// The identity of the Generation this machine was built from.
    pub candidate_id: [u8; 32],
    /// The immutable EROFS root, for its installation-time identity only.
    pub root: &'a mut File,
    /// The Instance-private overlay head, which becomes this snapshot's sterile template.
    pub overlay: &'a mut File,
    /// The console line the agent prints at the repair point.
    pub repair_point_line: Vec<u8>,
    /// How long the vCPU may take to leave `KVM_RUN` after it is kicked.
    pub grace: std::time::Duration,
}

/// What one published snapshot is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureOutcome {
    pub paths: SnapshotPaths,
    pub memory_digest: Digest,
    pub memory_bytes: u64,
    pub overlay_digest: Digest,
    pub overlay_bytes: u64,
    pub state_digest: Digest,
    pub state_bytes: u64,
    /// Identity of the immutable root the restored Instances share.
    pub root_digest: Digest,
    /// When the agent announced the repair point, relative to the machine's own clock.
    pub repair_point_at: Instant,
    /// Receive buffers the driver had posted at the capture point: network, vsock, events.
    pub posted_buffers: [u32; 3],
}

/// Captures `sandbox` at the repair point and publishes the snapshot.
///
/// The caller must have started the machine with the repair-point console line watched, and
/// must destroy the machine afterwards: a captured source machine is never resumed.
///
/// # Errors
///
/// Returns the first typed failure; nothing complete is published when any step fails.
#[allow(clippy::too_many_lines)]
pub fn capture(
    sandbox: &mut SandboxMachine,
    request: CaptureRequest<'_>,
    deadline: Instant,
) -> Result<CaptureOutcome, SnapshotError> {
    let mut quiesced = Quiesce::new();
    let repair_point_at =
        sandbox
            .wait_console_line(deadline)
            .ok_or(SnapshotError::NotQuiescent(
                "the guest agent never announced the disconnected repair point",
            ))?;
    // The watched line carries the pinned agent's own prefix, so it proves both that the
    // Generation's agent is the code that ran and that it parked in the launch-page wait
    // after flushing the private overlay.
    quiesced.prove(QuiescePrecondition::GenerationAgentBooted)?;
    quiesced.prove(QuiescePrecondition::RepairPointReached)?;
    quiesce::prove_no_ingress(&sandbox.bus())?;
    quiesced.prove(QuiescePrecondition::IngressDisabled)?;

    // `pause` joins the device thread first and only then kicks vCPU 0 out of `KVM_RUN`.
    sandbox.pause(request.grace)?;
    quiesced.prove(QuiescePrecondition::DeviceWorkDrained)?;
    quiesced.prove(QuiescePrecondition::OverlayFlushed)?;
    quiesced.prove(QuiescePrecondition::VcpuPaused)?;

    let paused = sandbox
        .paused()
        .ok_or(SnapshotError::NotQuiescent("the machine is not paused"))?;
    let mut bus = paused.bus;
    quiesce::drain(&mut bus, &paused.memory)?;
    quiesce::prove_queues_quiescent(&mut bus, &paused.memory)?;
    let posted = quiesce::posted(&mut bus, &paused.memory);
    quiesced.prove(QuiescePrecondition::QueuesProvenQuiescent)?;

    let mut sequence = quiesced.begin_capture()?;
    let vm_state = VmState::new(
        vec![MemorySlot {
            slot: 0,
            guest_address: 0,
            size: paused.ram_bytes,
            memory_offset: 0,
        }],
        layout::TSS_ADDRESS,
        0,
    )?;
    sequence.complete(CaptureStep::ReadVmState)?;
    let vcpu_state = vcpu::read(paused.kvm, paused.vcpu)?;
    sequence.complete(CaptureStep::ReadVcpuState)?;
    let irqchip = platform::read_irqchip(paused.vm)?;
    sequence.complete(CaptureStep::ReadIrqchip)?;
    let routing = platform::owned_routing();
    sequence.complete(CaptureStep::ReadIrqRouting)?;
    let clock = platform::read_clock(paused.vm)?;
    sequence.complete(CaptureStep::ReadClock)?;
    let pit = platform::read_pit(paused.vm)?;
    sequence.complete(CaptureStep::ReadPit)?;

    let live = bus.snapshot_all();
    let root_digest = artifacts::hash(Artifact::Root, request.root)?;
    let (_, cpu_template) = profile::cpu_template(paused.kvm)?;
    sequence.complete(CaptureStep::ReadDevices)?;

    let mut memory = Staging::create(&request.paths, Artifact::Memory, request.paths.memory())?;
    let mut buffer = vec![0_u8; CHUNK];
    let mut offset = 0_u64;
    while offset < paused.ram_bytes {
        let remaining = usize::try_from(paused.ram_bytes - offset).unwrap_or(CHUNK);
        let span = remaining.min(CHUNK);
        paused
            .memory
            .read_bytes(GuestAddress(offset), &mut buffer[..span])
            .map_err(|_| SnapshotError::NotQuiescent("guest RAM shrank during capture"))?;
        memory.write(&buffer[..span])?;
        offset += u64::try_from(span).unwrap_or(0);
    }
    let mut overlay = Staging::create(&request.paths, Artifact::Overlay, request.paths.overlay())?;
    overlay.write_file(request.overlay)?;
    let overlay_digest = overlay.running_digest();

    let devices = live
        .iter()
        .zip(Slot::ALL)
        .map(|(record, slot)| {
            let image = match slot {
                Slot::Root => root_digest,
                _ => overlay_digest,
            };
            let state = device::canonical(slot, record, device::specific(&bus, slot, image))?;
            if device::reproduces(slot, record, &state) {
                Ok(state)
            } else {
                Err(SnapshotError::DeviceStateNotCanonical(slot))
            }
        })
        .collect::<Result<Vec<DeviceState>, SnapshotError>>()?;
    drop(bus);

    let manifest = build(
        &request,
        &vm_state,
        &vcpu_state,
        &Parts {
            cpu_template,
            irqchip: &irqchip,
            routing: &routing,
            clock: &clock,
            pit: &pit,
            devices: &devices,
            memory: MemoryDescriptor::new(
                memory.running_digest(),
                memory.written(),
                PageSize::FOUR_KIB.get(),
            )?,
        },
    )?;
    let encoded = manifest.encode();
    let mut state = Staging::create(&request.paths, Artifact::State, request.paths.state())?;
    state.write(&encoded)?;
    sequence.complete(CaptureStep::WriteStagingObjects)?;

    if Manifest::decode(&encoded)? != manifest {
        return Err(SnapshotError::StagingNotCanonical);
    }
    sequence.complete(CaptureStep::IndependentlyDecodeStaging)?;

    let memory_digest = memory.seal()?;
    let overlay_digest = overlay.seal()?;
    let state_digest = state.seal()?;
    sequence.complete(CaptureStep::HashThroughRetainedHandles)?;

    let memory_bytes = memory.written();
    let overlay_bytes = overlay.written();
    let state_bytes = state.written();
    memory.link()?;
    overlay.link()?;
    state.link()?;
    artifacts::sync_directory(request.paths.directory())?;
    sequence.complete(CaptureStep::PublishGenerationManifest)?;

    Ok(CaptureOutcome {
        paths: request.paths,
        memory_digest,
        memory_bytes,
        overlay_digest,
        overlay_bytes,
        state_digest,
        state_bytes,
        root_digest,
        repair_point_at,
        posted_buffers: posted,
    })
}

struct Parts<'a> {
    cpu_template: Digest,
    irqchip: &'a crate::snapshot::kvm_state::IrqchipState,
    routing: &'a crate::snapshot::kvm_state::IrqRoutingState,
    clock: &'a crate::snapshot::kvm_state::ClockState,
    pit: &'a crate::snapshot::kvm_state::PitState,
    devices: &'a [DeviceState],
    memory: MemoryDescriptor,
}

fn build(
    request: &CaptureRequest<'_>,
    vm_state: &VmState,
    vcpu_state: &crate::snapshot::kvm_state::VcpuState,
    parts: &Parts<'_>,
) -> Result<Manifest, SnapshotError> {
    let header = ManifestHeader {
        architecture: Architecture::X86_64,
        page_size: PageSize::FOUR_KIB,
        candidate_id: CandidateId::new(request.candidate_id)?,
        machine_contract: profile::machine_contract(),
        device_contract: profile::device_contract(),
        cpu_template: parts.cpu_template,
        host: profile::requirements()?,
        memory: parts.memory,
        vcpu_count: profile::VCPU_COUNT,
        guest_protocol_version: profile::GUEST_PROTOCOL_VERSION,
    };
    let mut sections = vec![
        Section::new(SectionRole::VmState, vm_state.encode())?,
        Section::new(SectionRole::Vcpu0, vcpu_state.encode())?,
        Section::new(SectionRole::Irqchip, parts.irqchip.encode())?,
        Section::new(SectionRole::IrqRouting, parts.routing.encode())?,
        Section::new(SectionRole::KvmClock, parts.clock.encode())?,
        Section::new(SectionRole::Pit, parts.pit.encode())?,
    ];
    for (state, role) in parts.devices.iter().zip([
        SectionRole::Device0,
        SectionRole::Device1,
        SectionRole::Device2,
        SectionRole::Device3,
        SectionRole::Device4,
    ]) {
        sections.push(Section::new(role, state.encode())?);
    }
    sections.push(Section::new(
        SectionRole::RepairPointMarker,
        marker::encode(&request.repair_point_line),
    )?);
    Ok(Manifest::new(header, sections)?)
}
