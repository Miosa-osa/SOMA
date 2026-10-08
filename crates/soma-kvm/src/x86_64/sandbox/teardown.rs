//! Ordered teardown: reclaim every vCPU, stop the device thread, deregister every route, retire
//! a still-mapped launch page, release the VM and mappings, and assemble the evidence.

use std::{sync::PoisonError, time::Duration};

use super::{Milestone, Paused, Running, SandboxEvidence, SandboxMachine, Stage};
use crate::x86_64::{
    error::{MachineError, Phase},
    event_loop::EventLoopReport,
    mmio::MmioCounters,
    ports::{BusCounters, PortBusHandle},
    run::GuestExit,
    serial::SerialCounters,
    watchdog::{CANCELLATION_GRACE, RunReport, total_mmio},
};

const KERNEL_INIT_LINE: &[u8] = b"Run /init as init process";
const AGENT_READY_LINE: &[u8] = b"soma-guest-agent: ready";

impl SandboxMachine {
    /// Reclaims every vCPU within `exit_deadline`, stops the device thread, deregisters every
    /// route, releases every mapping and descriptor, and returns the evidence.
    pub fn finish(mut self, exit_deadline: Duration) -> SandboxEvidence {
        let (reports, devices, retired) = self.stop(exit_deadline);
        let ports = self.ports.take();
        let (serial, bus, uart, marks) = ports.and_then(PortBusHandle::into_inner).map_or_else(
            || {
                (
                    Vec::new(),
                    BusCounters::default(),
                    SerialCounters::default(),
                    Vec::new(),
                )
            },
            |bus| {
                let serial = bus.serial();
                let marks = vec![
                    (Milestone::KernelInit, serial.line_instant(KERNEL_INIT_LINE)),
                    (
                        Milestone::AgentReadyLine,
                        serial.line_instant(AGENT_READY_LINE),
                    ),
                ];
                (
                    serial.output().to_vec(),
                    bus.counters(),
                    bus.serial_counters(),
                    marks,
                )
            },
        );
        // Every dispatcher's counters are summed, so the evidence counts what the whole machine
        // did rather than what the bootstrap processor happened to do.
        let mmio: MmioCounters = total_mmio(&reports);
        let exit = machine_exit(&reports);
        let vcpus: Vec<Result<GuestExit, MachineError>> =
            reports.iter().map(|report| report.result.clone()).collect();
        let Self {
            machine,
            shared,
            clock,
            timeline,
            cmdline,
            entry,
            initramfs,
            exits,
            ..
        } = self;
        drop(shared);
        drop(machine);
        let mut timeline = timeline
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        for (milestone, at) in marks {
            if let Some(at) = at {
                timeline.mark_at(milestone, at);
            }
        }
        // The vCPU threads cannot mark the shared timeline while they run, so the two sides of
        // the machine's first `KVM_RUN` are transcribed here from the offsets it recorded.
        for (milestone, at) in [
            (Milestone::FirstRunEntered, exits.first_entry()),
            (Milestone::FirstRunReturned, exits.first_return()),
        ] {
            if let Some(at) = at {
                timeline.mark_at(milestone, at);
            }
        }
        timeline.mark(Milestone::Cleanup);
        let mut clock = clock;
        clock.lap(Phase::Cleanup);
        let (_, phases) = clock.finish();
        SandboxEvidence {
            serial,
            phases,
            timeline: timeline.finish(),
            cmdline,
            entry,
            initramfs,
            exit,
            vcpus,
            bus,
            uart,
            mmio,
            devices,
            launch_page_retired: retired,
            exits: exits.counts(),
        }
    }

    fn stop(&mut self, exit_deadline: Duration) -> (Vec<RunReport>, EventLoopReport, bool) {
        let stage = std::mem::replace(&mut self.stage, Stage::Stopped);
        let (reports, devices) = match stage {
            Stage::Running(running) => {
                let Running {
                    context,
                    vcpus,
                    event_loop,
                } = running;
                let mut reports = Vec::with_capacity(vcpus.len());
                let mut remaining = vcpus.into_iter();
                // The bootstrap processor decides when the machine has stopped. Every other
                // processor is interrupted the moment it does, because a guest that is already
                // going down leaves them idle inside KVM_RUN with nothing left to run.
                if let Some(bootstrap) = remaining.next() {
                    reports.push(bootstrap.wait(exit_deadline));
                }
                for run in remaining {
                    reports.push(run.pause(CANCELLATION_GRACE));
                }
                drop(context);
                self.mark(Milestone::GuestExit);
                self.clock.lap(Phase::Run);
                let devices = match event_loop.stop() {
                    Some((devices, mut notify, mut irq)) => {
                        notify.unregister(&self.machine.vm);
                        irq.unregister(&self.machine.vm);
                        devices
                    }
                    None => EventLoopReport::default(),
                };
                (reports, devices)
            }
            Stage::Prepared(prepared) => {
                let super::Prepared {
                    vcpus,
                    serial_line,
                    mut irq,
                    mut notify,
                } = prepared;
                drop(vcpus);
                notify.unregister(&self.machine.vm);
                irq.unregister(&self.machine.vm);
                drop(serial_line);
                (
                    never_ran("sandbox never started"),
                    EventLoopReport::default(),
                )
            }
            Stage::Paused(paused) => {
                let Paused { reports, devices } = *paused;
                (reports, devices)
            }
            Stage::Stopped => (
                never_ran("sandbox already stopped"),
                EventLoopReport::default(),
            ),
        };
        let retired = if self.launch_page_retired() {
            true
        } else {
            self.retire_launch_page().is_ok()
        };
        (reports, devices, retired)
    }
}

/// The machine's outcome: the first failure any vCPU reported, or the first exit.
fn machine_exit(reports: &[RunReport]) -> Result<GuestExit, MachineError> {
    let mut first = None;
    for report in reports {
        match &report.result {
            Err(error) => return Err(error.clone()),
            Ok(exit) if first.is_none() => first = Some(*exit),
            Ok(_) => {}
        }
    }
    first.ok_or_else(|| MachineError::invalid(Phase::Run, "no vCPU reported an exit"))
}

fn never_ran(reason: &'static str) -> Vec<RunReport> {
    vec![RunReport {
        mmio: None,
        result: Err(MachineError::invalid(Phase::Run, reason)),
        vcpu: None,
    }]
}
