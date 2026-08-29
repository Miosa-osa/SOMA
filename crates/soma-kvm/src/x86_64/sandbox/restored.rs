//! Assembling a sandbox from restored state instead of from a cold boot.
//!
//! Everything after this point - the control channel, the fresh launch page, the repair
//! commit, the milestones, and the ordered teardown - is the same code the cold-boot proof
//! uses, so a restored Instance cannot take a weaker path to `Ready` than a booted one.

use std::sync::{Arc, Mutex, atomic::AtomicBool};

use kvm_ioctls::VcpuFd;
use vmm_sys_util::eventfd::EventFd;

use super::{Prepared, SandboxMachine, Stage, Timeline};
use crate::virtio::MmioBus;
use crate::x86_64::{
    Machine,
    devices::SharedBus,
    error::{MachineError, Phase},
    events::{IrqLines, NotifyFds},
    launch_page::LaunchPageSlot,
    timing::Stopwatch,
};

/// Every owned resource a restore produced, in the order the machine will release them.
pub(in crate::x86_64) struct RestoredParts {
    pub(in crate::x86_64) machine: Machine,
    pub(in crate::x86_64) bus: MmioBus,
    pub(in crate::x86_64) vcpu: VcpuFd,
    pub(in crate::x86_64) serial_line: EventFd,
    pub(in crate::x86_64) irq: IrqLines,
    pub(in crate::x86_64) notify: NotifyFds,
    pub(in crate::x86_64) launch_page: LaunchPageSlot,
    pub(in crate::x86_64) clock: Stopwatch,
    pub(in crate::x86_64) timeline: Timeline,
    /// The exact command line the captured Generation was booted with.
    pub(in crate::x86_64) cmdline: String,
}

impl SandboxMachine {
    /// Wraps restored resources in the same machine the cold-boot path produces.
    ///
    /// # Errors
    ///
    /// Fails only when the host-work eventfd cannot be created.
    pub(in crate::x86_64) fn from_restored(parts: RestoredParts) -> Result<Self, MachineError> {
        let host_work = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Restore, &error))?;
        Ok(Self {
            machine: parts.machine,
            shared: Arc::new(SharedBus::new(parts.bus)),
            host_work: Arc::new(host_work),
            finished: Arc::new(AtomicBool::new(false)),
            launch_page: Mutex::new(Some(parts.launch_page)),
            console: None,
            stage: Stage::Prepared(Prepared {
                vcpu: parts.vcpu,
                serial_line: parts.serial_line,
                irq: parts.irq,
                notify: parts.notify,
            }),
            clock: parts.clock,
            timeline: Mutex::new(parts.timeline),
            cmdline: parts.cmdline,
            entry: 0,
            initramfs: None,
        })
    }
}
