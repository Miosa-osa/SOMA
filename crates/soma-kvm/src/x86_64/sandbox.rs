//! The test-only sandbox machine: one compiled Generation cold-booted for an authenticated
//! guest agent.
//!
//! `create` builds every owned resource in the contract order without running the guest;
//! `write_launch_page` publishes the material; `start` runs the device thread and vCPU 0;
//! the caller drives the byte-level [`ControlChannel`] with `soma-guest`, retires the launch
//! page at the repair commit, marks its own milestones, and `finish` reclaims the vCPU,
//! stops the device thread, deregisters every route, and returns the evidence.

pub(in crate::x86_64) mod evidence;
mod launch;
mod pause;
pub(in crate::x86_64) mod restored;
mod teardown;

use std::{
    fs::File,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};

use kvm_ioctls::VcpuFd;
use vmm_sys_util::eventfd::EventFd;

pub(in crate::x86_64) use self::evidence::Timeline;
pub use self::evidence::{Milestone, MilestoneMark, SandboxEvidence};
use super::{
    InterruptController, Machine, MachineError, Phase,
    channel::ControlChannel,
    cmdline,
    console_tap::ConsoleTap,
    devices::{self, DeviceIdentity, SandboxDisks, SharedBus},
    event_loop::EventLoop,
    events::{IrqLines, NotifyFds},
    launch_page::LaunchPageSlot,
    loader::{self, INITRAMFS_LIMIT, KERNEL_IMAGE_LIMIT},
    mmio::MmioDispatch,
    ports::PortBus,
    serial::{SERIAL_GSI, Serial},
    timing::Stopwatch,
    watchdog::VcpuRun,
};
use super::{event_loop::EventLoopReport, watchdog::RunReport};

/// Inputs for one sandbox.
pub struct SandboxConfig {
    /// The Generation's uncompressed PVH kernel.
    pub kernel: File,
    /// The Generation's `newc` initramfs carrying the guest agent.
    pub initramfs: File,
    /// The immutable root and the Instance-private overlay head.
    pub disks: SandboxDisks,
    /// Non-secret device identity for this Instance.
    pub identity: DeviceIdentity,
    /// Guest RAM in bytes; a multiple of 4 KiB between 128 MiB and 3 GiB.
    pub ram_bytes: u64,
}

struct Prepared {
    vcpu: VcpuFd,
    serial_line: EventFd,
    irq: IrqLines,
    notify: NotifyFds,
}

struct Running {
    vcpu: VcpuRun,
    event_loop: EventLoop,
}

/// A machine whose device thread stopped and whose vCPU left `KVM_RUN` with every resource
/// still owned, so a snapshot builder can read its state.
struct Paused {
    report: RunReport,
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
    stage: Stage,
    clock: Stopwatch,
    timeline: Mutex<Timeline>,
    cmdline: String,
    entry: u64,
    initramfs: Option<(u64, u64)>,
}

impl SandboxMachine {
    /// Creates the VM, RAM, platform, devices, launch page slot, loaded guest, vCPU, and
    /// every eventfd route, in that order, without running anything.
    ///
    /// # Errors
    ///
    /// Returns the typed phase failure; everything created before it is released in reverse.
    pub fn create(config: SandboxConfig) -> Result<Self, MachineError> {
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
        let bus = devices::build_bus(config.disks, config.identity)?;
        clock.lap(Phase::Devices);
        timeline.mark(Milestone::Devices);
        let launch_page = LaunchPageSlot::map_and_register(&machine.vm)?;
        clock.lap(Phase::LaunchPage);
        timeline.mark(Milestone::LaunchPageMapped);
        let line = cmdline::compose_generation();
        let loaded = loader::load_kernel(&mut machine.ram, &image, Some(&initramfs), &line)?;
        drop(image);
        clock.lap(Phase::LoadGuest);
        timeline.mark(Milestone::LoadGuest);
        let vcpu = machine.boot_vcpu(loaded.entry, &mut clock)?;
        timeline.mark(Milestone::Vcpu);
        let serial_line = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Events, &error))?;
        machine
            .vm
            .register_irqfd(&serial_line, SERIAL_GSI)
            .map_err(|error| MachineError::os(Phase::Events, error))?;
        let mut irq = IrqLines::create()?;
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
            stage: Stage::Prepared(Prepared {
                vcpu,
                serial_line,
                irq,
                notify,
            }),
            clock,
            timeline: Mutex::new(timeline),
            cmdline: loaded.cmdline,
            entry: loaded.entry,
            initramfs: loaded.initramfs,
        })
    }

    /// Starts the device thread and vCPU 0.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; a machine that fails to start is still fully reclaimable.
    pub fn start(&mut self) -> Result<(), MachineError> {
        let Stage::Prepared(prepared) = std::mem::replace(&mut self.stage, Stage::Stopped) else {
            return Err(MachineError::invalid(Phase::Run, "sandbox already started"));
        };
        let kicks = prepared.notify.kicks()?;
        let event_loop = EventLoop::spawn(
            Arc::clone(&self.shared),
            self.machine.ram.shared(),
            prepared.notify,
            prepared.irq,
            self.host_work
                .try_clone()
                .map_err(|error| MachineError::io(Phase::EventLoop, &error))?,
        )
        .map_err(|error| MachineError::io(Phase::EventLoop, &error))?;
        self.clock.lap(Phase::EventLoop);
        self.mark(Milestone::EventLoop);
        let dispatch = MmioDispatch::new(
            Arc::clone(&self.shared),
            self.machine.ram.shared(),
            kicks,
            Arc::clone(&self.finished),
        );
        let bus = PortBus::new(Serial::with_tap(
            Some(prepared.serial_line),
            self.console.clone(),
        ));
        let vcpu = VcpuRun::start(prepared.vcpu, bus, Some(dispatch), None).map_err(|report| {
            report
                .result
                .err()
                .unwrap_or_else(|| MachineError::invalid(Phase::Run, "vCPU failed to start"))
        })?;
        self.mark(Milestone::RunStart);
        self.stage = Stage::Running(Running { vcpu, event_loop });
        Ok(())
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
