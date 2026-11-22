//! Live capture of a running sandbox at the disconnected repair point.
//!
//! The machine is captured before any launch page is written, so the memory image contains no
//! Instance identity, no session, no key, and no network authority: there is nothing to scrub
//! and nothing that could be replayed into a second Instance. The quiesce proofs, the read
//! order, and the publication order are all driven through the codec's typed schedules, so a
//! step performed out of order is a typed failure rather than a silently unsafe snapshot.

use std::time::Instant;

use super::{
    artifacts::{self, Staging},
    device,
    error::{Artifact, SnapshotError},
    platform, profile, quiesce, vcpu,
};
use crate::snapshot::{
    Digest,
    capture::{CaptureStep, Quiesce, QuiescePrecondition},
    device_state::DeviceState,
    kvm_state::{MemorySlot, VmState},
    manifest::{Manifest, PageSize},
    memory::{MemoryDescriptor, MemoryError},
};
use crate::virtio::Slot;
use crate::x86_64::{
    cpuid::GuestMachine,
    error::{MachineError, Phase},
    layout::{self, GuestLayout},
    sandbox::SandboxMachine,
};

// Assembling the manifest out of what was read is beside this file: it touches no KVM and needs
// no ordering, so keeping it apart is what stops the read order below being read as part of it.
use manifest::{Parts, build};

mod manifest;
mod members;
mod request;

pub use request::{CaptureOutcome, CapturePoint, CaptureRequest};

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
    mut request: CaptureRequest<'_>,
    deadline: Instant,
) -> Result<CaptureOutcome, SnapshotError> {
    let mut quiesced = Quiesce::new();
    let capture_ceiling = request.contract.max_memory_bytes();
    let repair_point_at = match request.point {
        CapturePoint::ConsoleLine => {
            sandbox
                .wait_console_line(deadline)
                .ok_or(SnapshotError::NotQuiescent(
                    "the guest agent never announced the disconnected repair point",
                ))?
        }
        CapturePoint::Acknowledged => Instant::now(),
    };
    // The watched line carries the pinned agent's own prefix, so it proves both that the
    // Generation's agent is the code that ran and that it parked in the launch-page wait
    // after flushing the private overlay. A suspension proves the same two facts with the
    // agent's authenticated acknowledgement, which only the Instance's own agent can send.
    quiesced.prove(QuiescePrecondition::GenerationAgentBooted)?;
    quiesced.prove(QuiescePrecondition::RepairPointReached)?;
    let suspension = matches!(request.point, CapturePoint::Acknowledged);
    if suspension {
        sandbox
            .bus()
            .vsock_mut()
            .device_mut()
            .retire_for_suspension();
    }
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
    // The final drain may have taken the guest's last packets on the retired connection.
    if suspension {
        bus.vsock_mut().device_mut().retire_for_suspension();
    }
    quiesce::prove_queues_quiescent(&mut bus, &paused.memory)?;
    let posted = quiesce::posted(&mut bus, &paused.memory);
    quiesced.prove(QuiescePrecondition::QueuesProvenQuiescent)?;

    // A machine contract is what decides whether this much RAM may be captured at all. The
    // descriptor's own bound is the widest ceiling any contract admits, so without this a v1
    // machine could publish a sixteen-gigabyte memory object no v1 restore is allowed to map. The
    // check runs before the capture walk rather than at the descriptor, because the walk is what
    // costs the host a full write of that RAM.
    if !request.contract.accepts_memory(paused.ram_bytes) {
        return Err(MemoryError::SizeExceedsBound {
            size: paused.ram_bytes,
            bound: capture_ceiling,
        }
        .into());
    }
    let mut sequence = quiesced.begin_capture()?;
    // The certified memory-slot layout is derived from the captured RAM size, so a restore
    // re-derives the same ranges rather than trusting a list read out of the artifact.
    let layout = GuestLayout::new(paused.ram_bytes).map_err(MachineError::from)?;
    let slots = layout
        .regions()
        .iter()
        .map(|region| MemorySlot {
            slot: region.slot,
            guest_address: region.guest_start,
            size: region.size,
            memory_offset: region.host_offset,
        })
        .collect();
    let vm_state = VmState::new(slots, layout::TSS_ADDRESS, 0)?;
    sequence.complete(CaptureStep::ReadVmState)?;
    let vcpu_states = paused
        .vcpus
        .iter()
        .map(|vcpu| vcpu::read(paused.kvm, vcpu))
        .collect::<Result<Vec<_>, SnapshotError>>()?;
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
    // A Suspension resumes over the very root its Generation names, which the resume checks by
    // the Generation's identity, so re-hashing the root on every suspension would only cost time.
    let root_digest = if suspension {
        artifacts::UNHASHED
    } else {
        artifacts::hash(Artifact::Root, request.root)?
    };
    // A machine always has at least one processor and its contract admits at most eight, so the
    // count fits the field a template states it in; anything else is a machine this capture
    // cannot describe.
    let vcpus = u16::try_from(paused.vcpus.len()).map_err(|_| {
        MachineError::invalid(
            Phase::Cpuid,
            "the machine has more processors than a template can state",
        )
    })?;
    let (_, cpu_template) =
        profile::cpu_template(paused.kvm, GuestMachine::new(request.contract, vcpus))?;
    sequence.complete(CaptureStep::ReadDevices)?;

    let (mut memory, mut overlay) = members::write(
        &request.paths,
        (&paused.memory, paused.ram_bytes),
        request.overlay.take(),
        suspension,
    )?;
    let overlay_digest = overlay
        .as_ref()
        .map_or_else(|| Digest::from_bytes([0; 32]), Staging::running_digest);

    // Only the slots this machine has are captured, and each keeps its own section role, so a
    // manifest says which devices existed by which sections it carries.
    let devices = live
        .iter()
        .map(|record| {
            let slot = record.slot;
            let image = match slot {
                Slot::Root => root_digest,
                _ => overlay_digest,
            };
            let specific = device::specific(&bus, slot, image)
                .ok_or(SnapshotError::DeviceStateNotCanonical(slot))?;
            let state = device::canonical(&bus, slot, record, specific, request.contract)?;
            if device::reproduces(slot, record, &state) {
                Ok((slot, state))
            } else {
                Err(SnapshotError::DeviceStateNotCanonical(slot))
            }
        })
        .collect::<Result<Vec<(Slot, DeviceState)>, SnapshotError>>()?;
    let device_set = bus.device_set();
    drop(bus);

    let manifest = build(
        &request,
        &vm_state,
        &vcpu_states,
        &Parts {
            contract: request.contract,
            cpu_template,
            irqchip: &irqchip,
            routing: &routing,
            clock: &clock,
            pit: &pit,
            devices: &devices,
            device_set,
            hotplug: sandbox.hotplug_region(),
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
    let overlay_digest = overlay
        .as_mut()
        .map(Staging::seal)
        .transpose()?
        .unwrap_or_else(|| Digest::from_bytes([0; 32]));
    let state_digest = state.seal()?;
    sequence.complete(CaptureStep::HashThroughRetainedHandles)?;

    let memory_bytes = memory.written();
    let overlay_bytes = overlay.as_ref().map_or(0, Staging::written);
    let state_bytes = state.written();
    memory.link()?;
    if let Some(overlay) = overlay {
        overlay.link()?;
    }
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
