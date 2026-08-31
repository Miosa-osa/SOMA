//! Live restore of a captured snapshot into a fresh Instance.
//!
//! The order is the machine contract's twelve steps, driven through the codec's typed
//! schedule. Constant-size compatibility is decided before the memory object is mapped, the
//! memory object is mapped privately and never copied, every backend is fresh, eventfds and
//! interrupt routes exist before any captured interrupt state is armed, the fresh launch page
//! goes into its own slot which the snapshot never contained, and the vCPU resumes only after
//! every state constructor has succeeded.

mod devices;
mod handle;
mod readiness;
mod sections;
mod sterile;

pub use handle::Restored;

use std::{cell::Cell, fs::File};

use kvm_ioctls::Kvm;
use vmm_sys_util::eventfd::EventFd;

use self::{
    devices::{Identity, recreate_devices, verify},
    sections::{Sections, net_mac, section, vsock_cid},
};
use super::{
    artifacts::{self, SnapshotPaths},
    error::{Artifact, SnapshotError},
    marker, platform, profile, vcpu,
};
use crate::snapshot::{
    Digest, compatibility,
    manifest::Manifest,
    memory::PrivateMapping,
    restore::{RestoreSequence, RestoreStep},
    section::SectionRole,
};
use crate::virtio::{DeviceSet, Slot};
use crate::x86_64::sandbox::NetworkAttachment;
use crate::x86_64::{
    Machine,
    devices::SandboxDisks,
    error::{MachineError, Phase},
    events::{IrqLines, NotifyFds},
    launch_page::LaunchPageSlot,
    layout::GuestLayout,
    memory::{GuestRam, RamMapping},
    sandbox::{Milestone, SandboxMachine, Timeline, restored::RestoredParts},
    serial::SERIAL_GSI,
    timing::Stopwatch,
};
use sterile::SterileFacts;
pub use sterile::{Sterile, SterileRequest};

/// What a caller asks a restore to produce.
pub struct RestoreRequest {
    /// The published snapshot directory.
    pub paths: SnapshotPaths,
    /// The immutable root and, when the Generation declared writable storage, the
    /// Instance-private overlay head cloned from `overlay.raw`.
    pub disks: SandboxDisks,
    /// The optional devices this Generation declared; a snapshot of a different set is refused.
    pub devices: DeviceSet,
    /// The fresh vsock context identifier this Instance is assigned.
    pub guest_cid: u32,
    /// Guest RAM the caller expects, from the Generation shape rather than from the snapshot.
    pub memory_bytes: u64,
    /// Whether to re-hash the memory object and the overlay template before mapping.
    ///
    /// This is the installation and audit boundary, not the warm request path: it reads every
    /// byte of both objects.
    pub verify_artifacts: bool,
    /// The assigned network bundle, when this Instance was given one.
    pub network: Option<NetworkAttachment>,
}

/// What the restored machine is, taken from the manifest rather than from the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreFacts {
    /// The digest of the exact snapshot state object this Instance was restored from.
    pub snapshot: Digest,
    pub candidate_id: [u8; 32],
    pub memory_bytes: u64,
    /// The console line the agent printed at the capture point.
    pub repair_point_line: Vec<u8>,
    /// The placeholder MAC the snapshot carries; restore keeps it.
    pub mac: [u8; 6],
    /// The context identifier the captured machine held, replaced by the fresh assignment.
    pub captured_cid: u64,
    /// The fresh context identifier this Instance holds.
    pub guest_cid: u32,
}

/// Restores one Instance from a published snapshot.
///
/// # Errors
///
/// Returns the first typed failure; every resource acquired before it is released in reverse
/// ownership order as the partially built machine unwinds.
pub fn restore(request: RestoreRequest) -> Result<Restored, SnapshotError> {
    let RestoreRequest {
        paths,
        disks,
        devices,
        guest_cid,
        memory_bytes,
        verify_artifacts,
        network,
    } = request;
    let SandboxDisks { root, overlay } = disks;
    let overlay_capacity_bytes = overlay
        .as_ref()
        .map(|overlay| {
            overlay
                .metadata()
                .map(|metadata| metadata.len())
                .map_err(|error| SnapshotError::io(Artifact::Overlay, "metadata", &error))
        })
        .transpose()?;
    restore_sterile(SterileRequest {
        paths,
        root,
        overlay_capacity_bytes,
        devices,
        memory_bytes,
        verify_artifacts,
    })?
    .assign(overlay, guest_cid, network)
}

/// Restores one machine that holds no Instance authority yet.
///
/// # Errors
///
/// Returns the first typed failure; every resource acquired before it is released in reverse
/// ownership order as the partially built machine unwinds.
#[allow(clippy::too_many_lines)]
pub fn restore_sterile(request: SterileRequest) -> Result<Sterile, SnapshotError> {
    let SterileRequest {
        paths,
        root,
        overlay_capacity_bytes,
        devices,
        memory_bytes,
        verify_artifacts,
    } = request;
    let mut timeline = Timeline::new();
    let mut sequence = RestoreSequence::start();
    let kvm = Kvm::new().map_err(|error| MachineError::os(Phase::Restore, error))?;
    let state_bytes = artifacts::read_state(&paths.state())?;
    let snapshot = Digest::of(&state_bytes);
    let manifest = Manifest::decode(&state_bytes)?;
    // The device set comes from the Generation being launched, never from the snapshot: a set
    // read out of the artifact would agree with itself, and the point of the check is that the
    // machine this host means to build and the machine the snapshot describes are the same one.
    let profile = profile::host_profile(&kvm, memory_bytes, devices)?;
    compatibility::check(&profile, &manifest)?;
    let state = Sections::read(&manifest, devices)?;
    let repair_point_line = marker::decode(section(&manifest, SectionRole::RepairPointMarker)?)?;
    if verify_artifacts {
        verify(&paths, &manifest, &state)?;
    }
    sequence.complete(RestoreStep::ValidateManifest)?;
    timeline.mark(Milestone::ValidateManifest);

    let vm = kvm
        .create_vm()
        .map_err(|error| MachineError::os(Phase::Restore, error))?;
    sequence.complete(RestoreStep::CreateVm)?;
    timeline.mark(Milestone::CreateVm);

    let layout = GuestLayout::new(manifest.header().memory.size())?;
    let memory = File::open(paths.memory())
        .map_err(|error| SnapshotError::io(Artifact::Memory, "open", &error))?;
    let mapping = PrivateMapping::map(&memory, manifest.header().memory.size())?;
    let (base, len) = mapping.into_raw();
    let base = std::ptr::NonNull::new(base).ok_or(SnapshotError::Mapping(
        crate::snapshot::memory::MappingError::ZeroLength,
    ))?;
    // The machine now owns the range and unmaps it exactly once, after the VM is released.
    let ram = GuestRam::from_mapping(RamMapping::adopt(base, len), layout)?;
    drop(memory);
    sequence.complete(RestoreStep::MapMemoryPrivately)?;
    timeline.mark(Milestone::MapMemory);

    // The launch page is a separate slot the snapshot never contained, and it stays empty
    // until `write_launch_page` publishes the material just before the resume. It is added
    // while the VM still has no vCPU, because that is what a memory-slot addition costs; the
    // identical call after the vCPU exists costs two milliseconds. It is also bound before the
    // machine adopts the VM, so on any later failure the VM is released before this mapping
    // is, which is the ownership order `RamMapping` relies on.
    let launch_page = LaunchPageSlot::map_and_register(&vm)?;
    timeline.mark(Milestone::LaunchPageMapped);

    let machine = Machine::adopt(kvm, vm, ram);
    machine.register_certified_slots(&state.vm)?;
    sequence.complete(RestoreStep::RegisterMemorySlots)?;
    timeline.mark(Milestone::RegisterSlots);

    let mac = net_mac(state.slot(Slot::Net))?;
    let captured_cid = vsock_cid(state.slot(Slot::Vsock))?;
    machine.recreate_platform(&state.vm, &state.routing)?;
    timeline.mark(Milestone::Platform);
    let bus = recreate_devices(
        &machine,
        root,
        overlay_capacity_bytes,
        &state,
        &Identity { mac, captured_cid },
        devices,
    )?;
    sequence.complete(RestoreStep::RecreateIrqchipAndDevices)?;
    timeline.mark(Milestone::Devices);

    let vcpu = machine
        .vm_fd()
        .create_vcpu(0)
        .map_err(|error| MachineError::os(Phase::Restore, error))?;
    sequence.complete(RestoreStep::CreateVcpu)?;
    timeline.mark(Milestone::Vcpu);
    vcpu::write_configuration(machine.kvm_fd(), &vcpu, &state.vcpu)?;
    sequence.complete(RestoreStep::RestoreCpuidAndMsrs)?;
    vcpu::write_registers(machine.kvm_fd(), &vcpu, &state.vcpu)?;
    sequence.complete(RestoreStep::RestoreVcpuState)?;
    timeline.mark(Milestone::VcpuRestored);

    let serial_line = EventFd::new(libc::EFD_NONBLOCK)
        .map_err(|error| MachineError::io(Phase::Restore, &error))?;
    machine
        .vm_fd()
        .register_irqfd(&serial_line, SERIAL_GSI)
        .map_err(|error| MachineError::os(Phase::Restore, error))?;
    let mut irq = IrqLines::create(devices)?;
    irq.register(machine.vm_fd())?;
    let notify = NotifyFds::register(machine.vm_fd(), &bus)?;
    // Every route exists before any captured interrupt state is armed.
    platform::write_irqchip(machine.vm_fd(), &state.irqchip)?;
    platform::write_pit(machine.vm_fd(), state.pit)?;
    platform::write_clock(machine.vm_fd(), state.clock)?;
    sequence.complete(RestoreStep::RestoreDeviceAndInterruptState)?;
    timeline.mark(Milestone::Events);

    let machine = SandboxMachine::from_restored(RestoredParts {
        machine,
        bus,
        vcpu,
        serial_line,
        irq,
        notify,
        launch_page,
        clock: Stopwatch::new(),
        timeline,
        cmdline: crate::x86_64::cmdline::compose_generation(devices),
    })?;
    Ok(Sterile {
        machine,
        facts: SterileFacts {
            snapshot,
            candidate_id: *manifest.header().candidate_id.as_bytes(),
            memory_bytes: manifest.header().memory.size(),
            repair_point_line,
            mac,
            captured_cid,
        },
        sequence,
    })
}
