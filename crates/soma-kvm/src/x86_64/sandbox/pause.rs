//! Pausing a running sandbox so its state can be read, and the borrowed view of it.
//!
//! Pausing is not stopping: the device thread is joined, every vCPU is kicked out of `KVM_RUN`
//! at a point where KVM has already saved every architectural register, and every KVM object,
//! mapping, backend, and descriptor stays owned. Nothing here reads state; it only produces the
//! view that the snapshot builder reads through.

use std::{
    sync::{Arc, MutexGuard},
    time::{Duration, Instant},
};

use kvm_ioctls::{Kvm, VcpuFd, VmFd};

use super::{Paused, Running, SandboxMachine, Stage};
use crate::virtio::MmioBus;
use crate::x86_64::{
    console_tap::ConsoleTap,
    error::{MachineError, Phase},
    memory::SharedRam,
    run::GuestExit,
};

/// Everything a paused machine lends to a reader, in one borrow.
pub(in crate::x86_64) struct PausedMachine<'a> {
    /// The KVM handle, for host-capability and CPU-template questions.
    pub(in crate::x86_64) kvm: &'a Kvm,
    /// The VM, for interrupt-controller, timer, and clock state.
    pub(in crate::x86_64) vm: &'a VmFd,
    /// Every vCPU, in index order, outside `KVM_RUN` and run by no thread.
    pub(in crate::x86_64) vcpus: Vec<&'a VcpuFd>,
    /// Exactly the guest RAM registered for this machine.
    pub(in crate::x86_64) ram_bytes: u64,
    /// The checked device view of that RAM.
    pub(in crate::x86_64) memory: SharedRam,
    /// The five device models, with no other thread servicing them.
    pub(in crate::x86_64) bus: MutexGuard<'a, MmioBus>,
}

impl Stage {
    fn begin_pause(&mut self) -> Result<Option<Running>, MachineError> {
        match self {
            Self::Paused(paused) if paused.complete => return Ok(None),
            Self::Paused(_) => {
                return Err(MachineError::invalid(
                    Phase::Capture,
                    "previous pause did not complete",
                ));
            }
            Self::Running(_) => {}
            _ => {
                return Err(MachineError::invalid(
                    Phase::Capture,
                    "only a running sandbox can be paused",
                ));
            }
        }
        let Self::Running(running) = std::mem::replace(self, Self::Stopped) else {
            unreachable!("the running stage was checked before taking it")
        };
        Ok(Some(running))
    }
}

impl SandboxMachine {
    /// Watches the console for one fixed line while the guest runs.
    ///
    /// A snapshot builder needs to know that the guest agent reached its repair point while the
    /// machine is still up, and the captured console is only returned when it stops. The needle
    /// belongs to the guest agent's contract, so the caller supplies it; it must be installed
    /// before [`SandboxMachine::start`].
    pub fn watch_console(&mut self, needle: &[u8]) {
        self.console = Some(Arc::new(ConsoleTap::watching(needle)));
    }

    /// Waits for the watched console line, or the deadline.
    pub(in crate::x86_64) fn wait_console_line(&self, deadline: Instant) -> Option<Instant> {
        self.console.as_ref().and_then(|tap| tap.wait(deadline))
    }

    pub(in crate::x86_64) fn is_running(&self) -> bool {
        matches!(&*self.stage.borrow(), Stage::Running(_))
    }

    /// Stops guest execution while retaining the machine and its memory.
    ///
    /// The device worker joins and every vCPU leaves `KVM_RUN` before this returns success.
    /// A repeated pause succeeds without disturbing the paused machine.
    ///
    /// # Errors
    ///
    /// Fails if the machine is neither running nor successfully paused, the device worker
    /// fails to report, or a vCPU stopped for a different reason. The machine remains reclaimable.
    pub fn pause(&self, grace: Duration) -> Result<(), MachineError> {
        let running = self.stage.borrow_mut().begin_pause()?;
        let Some(running) = running else {
            return Ok(());
        };
        let Running {
            context,
            vcpus,
            event_loop,
        } = running;
        // Keep the routes registered after the device thread joins. The ioeventfds retain
        // notifications that arrived before the last vCPU left KVM_RUN so resume can service them.
        let devices = event_loop.park();
        let mut reports = Vec::with_capacity(vcpus.len());
        let mut every = devices.is_some() && !vcpus.is_empty();
        for run in vcpus {
            let report = run.pause(grace);
            every &= report.result == Ok(GuestExit::Paused)
                && report.vcpu.is_some()
                && report.mmio.is_some();
            reports.push(report);
        }
        *self.stage.borrow_mut() = Stage::Paused(Box::new(Paused {
            reports,
            devices,
            complete: every,
            context: Some(context),
        }));
        if every {
            Ok(())
        } else {
            Err(MachineError::invalid(
                Phase::Capture,
                "the machine did not report a complete pause",
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

    /// Borrows the stopped vCPUs while a caller still holds an Instance's control session.
    pub(in crate::x86_64) fn with_paused<T>(
        &self,
        read: impl FnOnce(PausedMachine<'_>) -> T,
    ) -> Option<T> {
        let stage = self.stage.borrow();
        let Stage::Paused(paused) = &*stage else {
            return None;
        };
        if !paused.complete {
            return None;
        }
        let vcpus = paused
            .reports
            .iter()
            .map(|report| report.vcpu.as_ref())
            .collect::<Option<_>>()?;
        Some(read(PausedMachine {
            kvm: &self.machine.kvm,
            vm: &self.machine.vm,
            vcpus,
            ram_bytes: self.machine.ram.layout().ram_bytes(),
            memory: self.machine.ram.shared(),
            bus: self.shared.lock(),
        }))
    }

    /// The borrowed view of a paused machine.
    pub(in crate::x86_64) fn paused(&mut self) -> Option<PausedMachine<'_>> {
        let Stage::Paused(paused) = self.stage.get_mut() else {
            return None;
        };
        if !paused.complete {
            return None;
        }
        let mut vcpus = Vec::with_capacity(paused.reports.len());
        for report in &paused.reports {
            vcpus.push(report.vcpu.as_ref()?);
        }
        Some(PausedMachine {
            kvm: &self.machine.kvm,
            vm: &self.machine.vm,
            vcpus,
            ram_bytes: self.machine.ram.layout().ram_bytes(),
            memory: self.machine.ram.shared(),
            bus: self.shared.lock(),
        })
    }
}

#[cfg(test)]
mod tests;
