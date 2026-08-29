//! Pausing a running sandbox so its state can be read, and the borrowed view of it.
//!
//! Pausing is not stopping: the device thread is joined, vCPU 0 is kicked out of `KVM_RUN` at
//! a point where KVM has already saved every architectural register, and every KVM object,
//! mapping, backend, and descriptor stays owned. Nothing here reads state; it only produces
//! the view that the snapshot builder reads through.

use std::{
    sync::{Arc, MutexGuard},
    time::{Duration, Instant},
};

use kvm_ioctls::{Kvm, VcpuFd, VmFd};

use super::{Paused, SandboxMachine, Stage};
use crate::virtio::MmioBus;
use crate::x86_64::{
    console_tap::ConsoleTap,
    error::{MachineError, Phase},
    event_loop::EventLoopReport,
    memory::SharedRam,
    run::GuestExit,
};

/// Everything a paused machine lends to a reader, in one borrow.
pub(in crate::x86_64) struct PausedMachine<'a> {
    /// The KVM handle, for host-capability and CPU-template questions.
    pub(in crate::x86_64) kvm: &'a Kvm,
    /// The VM, for interrupt-controller, timer, and clock state.
    pub(in crate::x86_64) vm: &'a VmFd,
    /// vCPU 0, outside `KVM_RUN` and run by no thread.
    pub(in crate::x86_64) vcpu: &'a VcpuFd,
    /// Exactly the guest RAM registered as memory slot 0 at guest-physical address 0.
    pub(in crate::x86_64) ram_bytes: u64,
    /// The checked device view of that RAM.
    pub(in crate::x86_64) memory: SharedRam,
    /// The five device models, with no other thread servicing them.
    pub(in crate::x86_64) bus: MutexGuard<'a, MmioBus>,
}

impl SandboxMachine {
    /// Watches the console for one fixed line while the guest runs.
    ///
    /// A snapshot builder needs to know that the guest agent reached its repair point while
    /// the machine is still up, and the captured console is only returned when it stops. The
    /// needle belongs to the guest agent's contract, so the caller supplies it; it must be
    /// installed before [`SandboxMachine::start`].
    pub fn watch_console(&mut self, needle: &[u8]) {
        self.console = Some(Arc::new(ConsoleTap::watching(needle)));
    }

    /// Waits for the watched console line, or the deadline.
    pub(in crate::x86_64) fn wait_console_line(&self, deadline: Instant) -> Option<Instant> {
        self.console.as_ref().and_then(|tap| tap.wait(deadline))
    }

    /// Stops the device thread, kicks vCPU 0 out of `KVM_RUN`, and deregisters every route.
    ///
    /// # Errors
    ///
    /// Fails when the machine is not running or when the vCPU stopped for any reason other
    /// than the pause; the machine remains fully reclaimable either way.
    pub(in crate::x86_64) fn pause(&mut self, grace: Duration) -> Result<(), MachineError> {
        let Stage::Running(running) = std::mem::replace(&mut self.stage, Stage::Stopped) else {
            return Err(MachineError::invalid(
                Phase::Capture,
                "only a running sandbox can be paused",
            ));
        };
        // The device thread joins first so no queue is serviced concurrently, then the vCPU
        // leaves KVM_RUN, and only then are the KVM routes removed: a guest notification that
        // races the join is absorbed by its ioeventfd and drained by the capture pass.
        let stopped = running.event_loop.stop();
        let report = running.vcpu.pause(grace);
        let devices = match stopped {
            Some((activity, mut notify, mut irq)) => {
                notify.unregister(&self.machine.vm);
                irq.unregister(&self.machine.vm);
                activity
            }
            None => EventLoopReport::default(),
        };
        let paused = report.result == Ok(GuestExit::Paused) && report.vcpu.is_some();
        self.stage = Stage::Paused(Box::new(Paused { report, devices }));
        if paused {
            Ok(())
        } else {
            Err(MachineError::invalid(
                Phase::Capture,
                "the vCPU stopped before it could be paused",
            ))
        }
    }

    /// Wakes the device thread so host-side work reaches the guest.
    ///
    /// A restored machine uses it to deliver the transport-reset event the vsock restore
    /// queued, which is what makes the guest driver re-read its fresh context identifier.
    pub(in crate::x86_64) fn wake_devices(&self) {
        let _ignored = self.host_work.write(1);
    }

    /// The five device models, whether the machine is running or paused.
    pub(in crate::x86_64) fn bus(&self) -> MutexGuard<'_, MmioBus> {
        self.shared.lock()
    }

    /// The borrowed view of a paused machine.
    pub(in crate::x86_64) fn paused(&self) -> Option<PausedMachine<'_>> {
        let Stage::Paused(paused) = &self.stage else {
            return None;
        };
        Some(PausedMachine {
            kvm: &self.machine.kvm,
            vm: &self.machine.vm,
            vcpu: paused.report.vcpu.as_ref()?,
            ram_bytes: self.machine.ram.layout().ram_bytes(),
            memory: self.machine.ram.shared(),
            bus: self.shared.lock(),
        })
    }
}
