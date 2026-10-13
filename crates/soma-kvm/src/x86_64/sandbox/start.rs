//! Starting a machine: the device thread, one thread per vCPU, and the one precondition that
//! refuses to run a guest at all.
//!
//! This is kept apart from the machine's construction because it is the last point at which a
//! device can still refuse to serve. A durable volume is declared against a placeholder and the
//! Instance's own file is bound over it before this point, so a machine that reached the run
//! still holding the placeholder is refused here rather than started with an empty disk.

use std::sync::Arc;

use super::{Milestone, Prepared, Running, SandboxMachine, Stage};
use crate::virtio::BlockConfigError;
use crate::x86_64::{
    error::{MachineError, MachineErrorKind, Phase},
    event_loop::EventLoop,
    mmio::MmioDispatch,
    ports::{PortBus, PortBusHandle},
    serial::Serial,
    watchdog::{CANCELLATION_GRACE, RunContext, VcpuRun},
};

impl SandboxMachine {
    /// Starts the device thread and one dedicated thread per vCPU.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; a machine that fails to start is still fully reclaimable.
    pub fn start(&mut self) -> Result<(), MachineError> {
        // Fail closed on the durable volume before anything runs: a declared volume is built on
        // a placeholder, and the Instance's own file is bound over it before the guest may run.
        // A machine that reached this point still holding the placeholder would give the guest
        // an empty disk where its data belongs, so it is refused and left fully reclaimable.
        if self
            .shared
            .lock()
            .volume()
            .is_some_and(|volume| volume.device().is_placeholder())
        {
            return Err(MachineError::new(
                Phase::Devices,
                MachineErrorKind::Block(BlockConfigError::PlaceholderSwapMissing),
            ));
        }
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
}
