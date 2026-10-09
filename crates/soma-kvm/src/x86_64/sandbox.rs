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
mod network;
mod pause;
pub(in crate::x86_64) mod restored;
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
    mmio::MmioDispatch,
    ports::{PortBus, PortBusHandle},
    serial::{SERIAL_GSI, Serial},
    timing::Stopwatch,
    watchdog::{CANCELLATION_GRACE, RunContext, RunReport, VcpuRun},
};

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
    cmdline: String,
    entry: u64,
    initramfs: Option<(u64, u64)>,
}

impl SandboxMachine {
    /// Creates the VM, RAM, platform, devices, launch page slot, guest, vCPUs, and eventfd
    /// routes, in that order, without running anything.
    ///
    /// # Errors
    ///
    /// Returns the typed phase failure; everything created before it is released in reverse.
    pub fn create(config: SandboxConfig) -> Result<Self, MachineError> {
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
        let bus = devices::build_bus(
            config.disks,
            config.identity,
            config.devices,
            config.contract,
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
            cmdline: loaded.cmdline,
            entry: loaded.entry,
            initramfs: loaded.initramfs,
        })
    }

    /// Starts the device thread and one dedicated thread per vCPU.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; a machine that fails to start is still fully reclaimable.
    pub fn start(&mut self) -> Result<(), MachineError> {
        let Stage::Prepared(prepared) = std::mem::replace(&mut self.stage, Stage::Stopped) else {
            return Err(MachineError::invalid(Phase::Run, "sandbox already started"));
        };
        let Prepared {
            vcpus,
            serial_line,
            irq,
            notify,
        } = prepared;
        // Each worker gets its own duplicated notify descriptors.
        let mut kicks = Vec::with_capacity(vcpus.len());
        for _ in 0..vcpus.len() {
            kicks.push(notify.kicks()?);
        }
        // An Instance with an assigned bundle has its TAP attached before this point, so the
        // device thread can watch it from its first wakeup.
        let event_loop = EventLoop::spawn(
            Arc::clone(&self.shared),
            self.machine.ram.shared(),
            notify,
            irq,
            self.host_work
                .try_clone()
                .map_err(|error| MachineError::io(Phase::EventLoop, &error))?,
            self.net_backend_fd(),
        )
        .map_err(|error| MachineError::io(Phase::EventLoop, &error))?;
        self.clock.lap(Phase::EventLoop);
        self.mark(Milestone::EventLoop);
        let ports = PortBusHandle::new(PortBus::new(Serial::with_tap(
            Some(serial_line),
            self.console.clone(),
        )));
        // RunStart is stamped BEFORE any vCPU thread exists: the timing contract reads
        // "armed <= entered", and every thread this loop spawns stamps FirstRunEntered on its
        // own way into KVM_RUN. Stamping after a spawn returns loses that order on a small
        // host - a 2-core machine was observed entering the guest 3.6us before the spawning
        // thread got to the mark.
        self.mark(Milestone::RunStart);
        let context = match RunContext::acquire() {
            Ok(context) => context,
            Err(error) => {
                let _ignored = event_loop.stop();
                return Err(error);
            }
        };
        let mut runs = Vec::with_capacity(vcpus.len());
        for (vcpu, kicks) in vcpus.into_iter().zip(kicks) {
            let dispatch = MmioDispatch::new(
                Arc::clone(&self.shared),
                self.machine.ram.shared(),
                kicks,
                Arc::clone(&self.finished),
            );
            match VcpuRun::start(&context, vcpu, &ports, Some(dispatch), None, &self.exits) {
                Ok(run) => runs.push(run),
                Err(report) => {
                    for run in runs {
                        let _ignored = run.pause(CANCELLATION_GRACE);
                    }
                    let _ignored = event_loop.stop();
                    return Err(report.result.err().unwrap_or_else(|| {
                        MachineError::invalid(Phase::Run, "vCPU failed to start")
                    }));
                }
            }
        }
        self.ports = Some(ports);
        self.stage = Stage::Running(Running {
            context,
            vcpus: runs,
            event_loop,
        });
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
