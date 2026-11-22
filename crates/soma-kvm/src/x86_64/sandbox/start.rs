//! Starting a machine: the device thread, one thread per vCPU, and the one precondition that
//! refuses to run a guest at all.
//!
//! This is kept apart from the machine's construction because it is the last point at which a
//! device can still refuse to serve. A durable volume is declared against a placeholder and the
//! Instance's own file is bound over it before this point, so a machine that reached the run
//! still holding the placeholder is refused here rather than started with an empty disk.

use std::sync::{Arc, atomic::Ordering};

use super::{Milestone, Prepared, Running, SandboxMachine, Stage};
use crate::virtio::BlockConfigError;
use crate::x86_64::{
    error::{MachineError, MachineErrorKind, Phase},
    event_loop::{DeviceRoutes, EventLoop},
    mmio::MmioDispatch,
    ports::{PortBus, PortBusHandle},
    serial::Serial,
    watchdog::{CANCELLATION_GRACE, RunContext, RunReport, VcpuRun},
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
        let placeholder = self
            .shared
            .lock()
            .volume()
            .is_some_and(|volume| volume.device().is_placeholder());
        if refuses_placeholder_volume(self.capture, placeholder) {
            return Err(MachineError::new(
                Phase::Devices,
                MachineErrorKind::Block(BlockConfigError::PlaceholderSwapMissing),
            ));
        }
        let stage = self.stage.get_mut();
        if !matches!(stage, Stage::Prepared(_)) {
            return Err(MachineError::invalid(Phase::Run, "sandbox already started"));
        }
        let Stage::Prepared(prepared) = std::mem::replace(stage, Stage::Stopped) else {
            unreachable!("the prepared stage was checked before taking it")
        };
        let Prepared {
            vcpus,
            serial_line,
            irq,
            notify,
        } = prepared;
        let workers = vcpus
            .into_iter()
            .map(|vcpu| {
                let dispatch = MmioDispatch::new(
                    Arc::clone(&self.shared),
                    self.machine.ram.shared(),
                    notify.kicks()?,
                    Arc::clone(&self.finished),
                );
                Ok((vcpu, dispatch))
            })
            .collect::<Result<Vec<_>, MachineError>>()?;
        let ports = PortBusHandle::new(PortBus::new(Serial::with_tap(
            Some(serial_line),
            self.console.clone(),
        )));
        let event_loop = self.start_devices(DeviceRoutes { notify, irq })?;
        self.clock.lap(Phase::EventLoop);
        self.mark(Milestone::EventLoop);
        // The timing contract requires this mark before any vCPU enters KVM_RUN.
        self.mark(Milestone::RunStart);
        let context = RunContext::acquire()?;
        self.start_vcpus(context, event_loop, workers, |context, (vcpu, dispatch)| {
            VcpuRun::start(context, vcpu, &ports, Some(dispatch), None, &self.exits)
        })?;
        self.ports = Some(ports);
        Ok(())
    }

    /// Restarts service threads over the same paused vCPUs, memory and devices.
    ///
    /// Repeating resume on a running machine does not create more workers.
    ///
    /// # Errors
    ///
    /// Returns an error if the machine never paused successfully or a worker cannot restart.
    /// A failed restart is reclaimable.
    pub fn resume(&self) -> Result<(), MachineError> {
        let mut stage = self.stage.borrow_mut();
        match &*stage {
            Stage::Running(_) => return Ok(()),
            Stage::Paused(paused) if paused.complete => {}
            _ => return Err(MachineError::invalid(Phase::Run, "sandbox is not paused")),
        }
        let ports = self
            .ports
            .clone()
            .ok_or_else(|| MachineError::invalid(Phase::Run, "paused sandbox lost its port bus"))?;
        let Stage::Paused(paused) = std::mem::replace(&mut *stage, Stage::Stopped) else {
            unreachable!("the paused stage was checked before taking it")
        };
        drop(stage);
        let devices = paused.devices.ok_or_else(|| {
            MachineError::invalid(Phase::Run, "paused sandbox lost its device loop")
        })?;
        let context = paused
            .context
            .ok_or_else(|| MachineError::invalid(Phase::Run, "paused sandbox lost its watchdog"))?;
        let event_loop = devices
            .resume()
            .map_err(|error| MachineError::io(Phase::EventLoop, &error))?;
        self.start_vcpus(context, event_loop, paused.reports, |context, report| {
            let RunReport { vcpu, mmio, .. } = report;
            match (vcpu, mmio) {
                (Some(vcpu), Some(dispatch)) => {
                    VcpuRun::resume(context, vcpu, &ports, dispatch, &self.exits)
                }
                _ => Err(MachineError::invalid(Phase::Run, "paused sandbox lost a vCPU").into()),
            }
        })?;
        // A network edge may have been dequeued in the same epoll batch as the stop wakeup.
        // Rescan the existing backend once so that edge does not need another packet to arrive.
        self.wake_devices();
        Ok(())
    }

    fn start_devices(&self, routes: DeviceRoutes) -> Result<EventLoop, MachineError> {
        // An Instance with an assigned bundle has its TAP attached before this point, so the
        // device thread can watch it from its first wakeup.
        EventLoop::spawn(
            Arc::clone(&self.shared),
            self.machine.ram.shared(),
            routes,
            self.host_work
                .try_clone()
                .map_err(|error| MachineError::io(Phase::EventLoop, &error))?,
            self.net_backend_fd(),
        )
        .map_err(|error| MachineError::io(Phase::EventLoop, &error))
    }

    fn start_vcpus<T>(
        &self,
        context: RunContext,
        event_loop: EventLoop,
        workers: Vec<T>,
        spawn: impl Fn(&RunContext, T) -> Result<VcpuRun, RunReport>,
    ) -> Result<(), MachineError> {
        self.finished.store(false, Ordering::Release);
        let mut runs = Vec::with_capacity(workers.len());
        for worker in workers {
            match spawn(&context, worker) {
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
        *self.stage.borrow_mut() = Stage::Running(Running {
            context,
            vcpus: runs,
            event_loop,
        });
        Ok(())
    }
}

/// Whether a machine that reached the run still holding its placeholder must be refused.
///
/// A serving guest must never read an empty disk where its files belong, so a launch's machine is
/// refused. A capture's machine is not a serving guest: the guest sees a device with no capacity
/// and skips the volume steps, so nothing is mounted and the snapshot holds no cached state from
/// the volume, which is exactly the invariant that lets the volume stay a placeholder here.
const fn refuses_placeholder_volume(capture: bool, placeholder: bool) -> bool {
    !capture && placeholder
}

#[cfg(test)]
mod tests {
    use super::refuses_placeholder_volume;

    #[test]
    fn a_serving_start_refuses_a_volume_still_on_its_placeholder() {
        assert!(refuses_placeholder_volume(false, true));
    }

    #[test]
    fn a_capture_start_accepts_the_placeholder_it_will_publish() {
        assert!(!refuses_placeholder_volume(true, true));
    }

    #[test]
    fn a_start_with_a_bound_volume_is_allowed_either_way() {
        assert!(!refuses_placeholder_volume(false, false));
        assert!(!refuses_placeholder_volume(true, false));
    }
}
