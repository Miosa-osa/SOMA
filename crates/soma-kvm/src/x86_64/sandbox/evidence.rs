//! Retained evidence of one sandbox run: milestones on one monotonic clock, the KVM phase
//! durations, the console capture, and every bounded counter.

use std::time::Instant;

use super::super::{
    event_loop::EventLoopReport, exits::ExitCounts, mmio::MmioCounters, ports::BusCounters,
    run::GuestExit, serial::SerialCounters, timing::PhaseTiming,
};

/// One named point on the sandbox timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Milestone {
    /// A restore validated the constant-size manifest identity and compatibility metadata.
    ValidateManifest,
    /// The KVM VM exists.
    CreateVm,
    /// Guest RAM is mapped and registered.
    MapRegister,
    /// A restore mapped the immutable memory object privately, without copying it.
    MapMemory,
    /// A restore walked the whole memory image, on an operator's diagnostic request.
    PrefaultMemory,
    /// A restore registered every certified memory slot.
    RegisterSlots,
    /// TSS window, in-kernel irqchip, and PIT exist.
    Platform,
    /// The five device models are bound to the bus.
    Devices,
    /// The launch page slot is mapped and registered.
    LaunchPageMapped,
    /// Kernel, initramfs, and boot pages are in guest RAM.
    LoadGuest,
    /// vCPU 0 exists with its CPUID template and bootstrap registers.
    Vcpu,
    /// A restore installed every certified vCPU state group.
    VcpuRestored,
    /// Every irqfd and ioeventfd is registered.
    Events,
    /// The launch material is in the slot.
    LaunchPageWritten,
    /// The device thread is serving.
    EventLoop,
    /// vCPU 0 is armed: its worker thread exists and has installed its run mask.
    RunStart,
    /// The vCPU worker was about to make its first `KVM_RUN` call.
    FirstRunEntered,
    /// The first `KVM_RUN` call returned.
    FirstRunReturned,
    /// The kernel handed control to `/init` (from the console line's host timestamp).
    KernelInit,
    /// The guest overwrote the launch page domain (host-side poll).
    LaunchPageConsumed,
    /// The guest agent's connection on the control port is open.
    VsockConnected,
    /// The caller completed the authenticated handshake.
    Handshake,
    /// The caller observed the authenticated `RepairComplete` and the page was retired.
    LaunchPageRetired,
    /// The caller completed the fixed readiness probe.
    Ready,
    /// The guest agent reported `ready` on its console (host timestamp of that line).
    AgentReadyLine,
    /// One bounded command round-trip completed.
    Execute,
    /// The authenticated shutdown was acknowledged.
    Shutdown,
    /// vCPU 0 stopped.
    GuestExit,
    /// Every thread, descriptor, route, and mapping was released.
    Cleanup,
}

/// One milestone with its offset from the moment the sandbox started being created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MilestoneMark {
    pub milestone: Milestone,
    pub elapsed_ns: u64,
}

/// The monotonic timeline of one sandbox.
pub(in crate::x86_64) struct Timeline {
    started: Instant,
    marks: Vec<MilestoneMark>,
}

impl Timeline {
    pub(in crate::x86_64) fn new() -> Self {
        Self {
            started: Instant::now(),
            marks: Vec::new(),
        }
    }

    pub(crate) fn mark(&mut self, milestone: Milestone) {
        self.mark_at(milestone, Instant::now());
    }

    pub(crate) fn mark_at(&mut self, milestone: Milestone, at: Instant) {
        self.marks.push(MilestoneMark {
            milestone,
            elapsed_ns: super::super::timing::saturating_ns(at.duration_since(self.started)),
        });
    }

    pub(crate) fn finish(mut self) -> Vec<MilestoneMark> {
        self.marks.sort_by_key(|mark| mark.elapsed_ns);
        self.marks
    }
}

/// Everything retained from one sandbox run.
#[derive(Clone, Debug)]
pub struct SandboxEvidence {
    /// Every byte the guest wrote to the diagnostic console.
    pub serial: Vec<u8>,
    /// KVM creation phases as durations, in lifecycle order.
    pub phases: Vec<PhaseTiming>,
    /// Milestones in time order with offsets from creation start.
    pub timeline: Vec<MilestoneMark>,
    /// The exact command line written to the guest.
    pub cmdline: String,
    /// The validated PVH entry point.
    pub entry: u64,
    /// Guest-physical start and size of the initramfs.
    pub initramfs: Option<(u64, u64)>,
    /// How the guest stopped, or why the run failed.
    pub exit: Result<GuestExit, super::super::MachineError>,
    /// Port-access counts by device.
    pub bus: BusCounters,
    /// Register-access counts inside the 16550 model.
    pub uart: SerialCounters,
    /// MMIO exits dispatched by the vCPU thread.
    pub mmio: MmioCounters,
    /// What the device thread did per slot.
    pub devices: EventLoopReport,
    /// Whether the launch page was verified erased and its slot removed.
    pub launch_page_retired: bool,
    /// Both sides of every `KVM_RUN` vCPU 0 made.
    pub exits: ExitCounts,
}

impl SandboxEvidence {
    /// Offset of the first mark for `milestone`, if it was recorded.
    #[must_use]
    pub fn at(&self, milestone: Milestone) -> Option<u64> {
        self.timeline
            .iter()
            .find(|mark| mark.milestone == milestone)
            .map(|mark| mark.elapsed_ns)
    }
}
