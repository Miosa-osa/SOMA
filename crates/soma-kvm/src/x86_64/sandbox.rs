//! The test-only sandbox machine: one compiled Generation cold-booted for an authenticated
//! guest agent.
//!
//! `create` builds every owned resource in the contract order without running the guest;
//! `write_launch_page` publishes the material; `start` runs the device thread and one thread
//! per vCPU; the caller drives the byte-level [`ControlChannel`] with `soma-guest`, retires the
//! launch page at the repair commit, marks its milestones, and `finish` reclaims every vCPU.

mod config;
pub(in crate::x86_64) mod evidence;
mod launch;
mod memory;
mod network;
mod pause;
pub(in crate::x86_64) mod restored;
mod start;
mod teardown;

use std::sync::{Arc, Mutex, PoisonError, atomic::AtomicBool};

use kvm_ioctls::VcpuFd;
use vmm_sys_util::eventfd::EventFd;

pub use self::config::SandboxConfig;
pub(in crate::x86_64) use self::evidence::Timeline;
pub use self::evidence::{Milestone, MilestoneMark, SandboxEvidence};
pub use self::restored::NetworkAttachment;
use super::{
    InterruptController, Machine, MachineError, Phase,
    channel::ControlChannel,
    cmdline,
    console_tap::ConsoleTap,
    cpuid,
    devices::{self, SharedBus},
    event_loop::{EventLoop, EventLoopReport},
    events::{IrqLines, NotifyFds},
    exits::ExitLedger,
    launch_page::LaunchPageSlot,
    loader::{self, INITRAMFS_LIMIT, KERNEL_IMAGE_LIMIT},
    ports::PortBusHandle,
    serial::SERIAL_GSI,
    timing::Stopwatch,
    watchdog::{RunContext, RunReport, VcpuRun},
};
use crate::head_reads::{HeadReadsHandle, HeadSpan};

struct Prepared {
    vcpus: Vec<VcpuFd>,
    serial_line: EventFd,
    irq: IrqLines,
    notify: NotifyFds,
}

struct Running {
    context: RunContext,
    vcpus: Vec<VcpuRun>,
    event_loop: EventLoop,
}

/// A machine whose device thread stopped and whose vCPUs left `KVM_RUN`, every resource owned.
struct Paused {
    reports: Vec<RunReport>,
    devices: EventLoopReport,
}

enum Stage {
    Prepared(Prepared),
    Running(Running),
    Paused(Box<Paused>),
    Stopped,
}

/// One owned sandbox machine.
pub struct SandboxMachine {
    machine: Machine,
    shared: Arc<SharedBus>,
    host_work: Arc<EventFd>,
    finished: Arc<AtomicBool>,
    launch_page: Mutex<Option<LaunchPageSlot>>,
    console: Option<Arc<ConsoleTap>>,
    /// The shared port bus, one console buffer whatever the vCPU count, taken back at the end.
    ports: Option<PortBusHandle>,
    /// Both sides of every `KVM_RUN` this machine's vCPUs make.
    exits: Arc<ExitLedger>,
    stage: Stage,
    clock: Stopwatch,
    timeline: Mutex<Timeline>,
    /// Where the ranges a guest reads from this machine's head are recorded, when a session asked
    /// to be told them. Nothing watches a machine that was not asked to be watched.
    head_reads: Mutex<Option<HeadReadsHandle>>,
    cmdline: String,
    entry: u64,
    initramfs: Option<(u64, u64)>,
    /// Whether this machine exists to be captured rather than to serve a guest.
    ///
    /// A capture's machine is built over the placeholder its Generation declares, and the run
    /// check that protects a serving guest from an empty disk is skipped for it: the guest sees a
    /// device with no capacity and skips the volume steps, so nothing is mounted and the snapshot
    /// holds no cached state from the volume.
    capture: bool,
}

impl SandboxMachine {
    /// Has this machine record the ranges a guest reads from its head.
    pub(in crate::x86_64) fn watch_head_reads(&self, reads: HeadReadsHandle) {
        *self
            .head_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reads);
    }

    /// Seals the head recorder and reports the ranges the guest read before Ready.
    ///
    /// Read once, where the launch reaches Ready, so that what the guest later *chooses* to read is
    /// not mistaken for a cost of the launch. Empty when nothing was watching.
    #[must_use]
    pub fn seal_head_reads(&self) -> Vec<HeadSpan> {
        let watching = self
            .head_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        watching.map_or_else(Vec::new, |reads| crate::head_reads::seal(&reads))
    }

    /// Creates the VM, RAM, platform, devices, launch page slot, guest, vCPUs, and eventfd
    /// routes, in that order, without running anything.
    ///
    /// # Errors
    ///
    /// Returns the typed phase failure; everything created before it is released in reverse.
    pub fn create(config: SandboxConfig) -> Result<Self, MachineError> {
        let capture = config.disks.volume_placeholder;
        if !config.contract.accepts_vcpus(config.vcpus) {
            return Err(MachineError::invalid(
                Phase::CreateVcpu,
                "the vCPU count is not one this machine contract admits",
            ));
        }
        if !config.contract.accepts_memory(config.ram_bytes) {
            return Err(MachineError::invalid(
                Phase::MapMemory,
                "the guest RAM size is not one this machine contract admits",
            ));
        }
        let mut timeline = Timeline::new();
        let mut clock = Stopwatch::new();
        let image = loader::read_bounded(config.kernel, KERNEL_IMAGE_LIMIT)?;
        let initramfs = loader::read_bounded(config.initramfs, INITRAMFS_LIMIT)?;
        clock.lap(Phase::ReadKernel);
        let mut machine = Machine::create(config.ram_bytes, &mut clock)?;
        timeline.mark(Milestone::CreateVm);
        timeline.mark(Milestone::MapRegister);
        machine.configure_platform(InterruptController::InKernel, true, &mut clock)?;
        timeline.mark(Milestone::Platform);
        // The region is placed and registered before the device that describes it exists, because
        // the device needs the backing over it and the machine, not the device table, owns that.
        let hotplug = match config.hotplug {
            Some(region) => {
                machine.ram.attach_hotplug(region)?;
                let ram = machine.ram.hotplug().ok_or_else(|| {
                    MachineError::invalid(Phase::MapMemory, "the hotplug region was not attached")
                })?;
                ram.register(&machine.vm)?;
                let backing: Box<dyn crate::virtio::RegionBacking + Send> =
                    Box::new(crate::x86_64::HotplugBacking::new(ram));
                Some((region.guest_start, region.size, backing))
            }
            None => None,
        };
        let bus = devices::build_bus(
            config.disks,
            config.identity,
            config.devices,
            config.contract,
            hotplug,
        )?;
        clock.lap(Phase::Devices);
        timeline.mark(Milestone::Devices);
        let launch_page = LaunchPageSlot::map_and_register(&machine.vm)?;
        clock.lap(Phase::LaunchPage);
        timeline.mark(Milestone::LaunchPageMapped);
        let line = cmdline::compose_generation_for(config.devices, config.contract);
        let loaded = loader::load_kernel(
            &mut machine.ram,
            &image,
            Some(&initramfs),
            &line,
            config.contract,
            config.vcpus,
        )?;
        drop(image);
        clock.lap(Phase::LoadGuest);
        timeline.mark(Milestone::LoadGuest);
        let shape = cpuid::GuestMachine::new(config.contract, config.vcpus);
        let vcpus = machine.boot_vcpus(loaded.entry, config.vcpus, shape, &mut clock)?;
        timeline.mark(Milestone::Vcpu);
        let serial_line = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Events, &error))?;
        machine
            .vm
            .register_irqfd(&serial_line, SERIAL_GSI)
            .map_err(|error| MachineError::os(Phase::Events, error))?;
        let mut irq = IrqLines::create(config.devices)?;
        irq.register(&machine.vm)?;
        let notify = NotifyFds::register(&machine.vm, &bus)?;
        clock.lap(Phase::Events);
        timeline.mark(Milestone::Events);
        let host_work = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Events, &error))?;
        Ok(Self {
            machine,
            shared: Arc::new(SharedBus::new(bus)),
            host_work: Arc::new(host_work),
            finished: Arc::new(AtomicBool::new(false)),
            launch_page: Mutex::new(Some(launch_page)),
            console: None,
            ports: None,
            exits: Arc::new(ExitLedger::new()),
            stage: Stage::Prepared(Prepared {
                vcpus,
                serial_line,
                irq,
                notify,
            }),
            clock,
            timeline: Mutex::new(timeline),
            head_reads: Mutex::new(None),
            cmdline: loaded.cmdline,
            entry: loaded.entry,
            initramfs: loaded.initramfs,
            capture,
        })
    }

    /// Adds the empty hotplug region behind its own KVM memory slot and returns the backing the
    /// memory device hands unplugged blocks to.
    ///
    /// Call it before [`Self::start`]. Boot RAM, the memory object, and the certified slot list
    /// are untouched; the region is neither captured nor restored as memory.
    ///
    /// # Errors
    ///
    /// Fails when the machine already has a region or the host refuses the mapping or slot.
    pub fn attach_hotplug(
        &mut self,
        region: crate::memory_layout::HotplugRegion,
    ) -> Result<crate::x86_64::HotplugBacking, MachineError> {
        if self.machine.ram.hotplug().is_some() {
            return Err(MachineError::invalid(
                Phase::MapMemory,
                "the machine already has a hotplug region",
            ));
        }
        self.machine.ram.attach_hotplug(region)?;
        let ram = self.machine.ram.hotplug().ok_or_else(|| {
            MachineError::invalid(Phase::MapMemory, "the hotplug region was not attached")
        })?;
        ram.register(&self.machine.vm)?;
        Ok(crate::x86_64::HotplugBacking::new(ram))
    }

    /// The backing over the machine's hotplug region, for a machine built with one or restored
    /// from a snapshot that carried one. Every call shares the same mapping, so the memory
    /// device can be rebuilt from it after a restore.
    #[must_use]
    pub fn hotplug_backing(&self) -> Option<crate::x86_64::HotplugBacking> {
        self.machine.ram.hotplug_backing()
    }

    /// The hotplug region, when the machine has one.
    #[must_use]
    pub fn hotplug_region(&self) -> Option<crate::memory_layout::HotplugRegion> {
        self.machine.ram.hotplug().map(|ram| ram.region())
    }

    /// The byte channel over the guest's control connection.
    #[must_use]
    pub fn control(&self) -> ControlChannel {
        ControlChannel::new(
            Arc::clone(&self.shared),
            Arc::clone(&self.host_work),
            Arc::clone(&self.finished),
        )
    }

    /// Records a caller-observed milestone.
    pub fn mark(&self, milestone: Milestone) {
        self.timeline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .mark(milestone);
    }

    /// The exact command line written to the guest.
    #[must_use]
    pub fn cmdline(&self) -> &str {
        &self.cmdline
    }
}
