//! Ordered teardown: reclaim vCPU 0, stop the device thread, deregister every route, retire a
//! still-mapped launch page, release the VM and mappings, and assemble the evidence.

use std::{sync::PoisonError, time::Duration};

use super::{Milestone, SandboxEvidence, SandboxMachine, Stage};
use crate::x86_64::{
    error::{MachineError, Phase},
    event_loop::EventLoopReport,
    mmio::MmioCounters,
    ports::BusCounters,
    serial::SerialCounters,
    watchdog::{CANCELLATION_GRACE, RunReport},
};

const KERNEL_INIT_LINE: &[u8] = b"Run /init as init process";
const AGENT_READY_LINE: &[u8] = b"soma-guest-agent: ready";

impl SandboxMachine {
    /// Stops the guest for good while it still owns everything.
    ///
    /// The device thread is joined and vCPU 0 is kicked out of `KVM_RUN` and never re-entered,
    /// so once this returns the guest cannot execute another instruction, whatever stage the
    /// machine was in. The VM, its memory, and its descriptors are still held and are released by
    /// [`SandboxMachine::finish`], which is the expensive half: closing the VM waits on KVM tearing
    /// down every guest mapping. A caller that must answer promptly halts, answers, and finishes
    /// afterwards.
    ///
    /// A vCPU that does not leave `KVM_RUN` within the cancellation grace aborts the process
    /// rather than leaving a live vCPU behind, exactly as a cancelled run does.
    pub fn halt(&mut self) {
        // A machine that is not running has no vCPU in `KVM_RUN`, which is the whole guarantee,
        // so the refusal that stage produces says nothing a caller needs.
        let _ignored = self.pause(CANCELLATION_GRACE);
    }

    /// Reclaims the vCPU within `exit_deadline`, stops the device thread, deregisters every
    /// route, releases every mapping and descriptor, and returns the evidence.
    pub fn finish(mut self, exit_deadline: Duration) -> SandboxEvidence {
        let (report, devices, retired) = self.stop(exit_deadline);
        let (serial, bus, uart, marks) = report.bus.as_ref().map_or_else(
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
        let mmio = report
            .mmio
            .as_ref()
            .map_or(MmioCounters::default(), |dispatch| dispatch.counters());
        let RunReport { result, .. } = report;
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
        // The vCPU thread cannot mark the shared timeline while it runs, so the two sides of
        // its first `KVM_RUN` are transcribed here from the offsets it recorded instead.
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
            exit: result,
            bus,
            uart,
            mmio,
            devices,
            launch_page_retired: retired,
            exits: exits.counts(),
        }
    }

    fn stop(&mut self, exit_deadline: Duration) -> (RunReport, EventLoopReport, bool) {
        let stage = std::mem::replace(&mut self.stage, Stage::Stopped);
        let (report, devices) = match stage {
            Stage::Running(running) => {
                let report = running.vcpu.wait(exit_deadline);
                self.mark(Milestone::GuestExit);
                self.clock.lap(Phase::Run);
                let devices = match running.event_loop.stop() {
                    Some((devices, mut notify, mut irq)) => {
                        notify.unregister(&self.machine.vm);
                        irq.unregister(&self.machine.vm);
                        devices
                    }
                    None => EventLoopReport::default(),
                };
                (report, devices)
            }
            Stage::Prepared(prepared) => {
                let super::Prepared {
                    vcpu,
                    serial_line,
                    mut irq,
                    mut notify,
                } = prepared;
                drop(vcpu);
                notify.unregister(&self.machine.vm);
                irq.unregister(&self.machine.vm);
                drop(serial_line);
                (
                    never_ran("sandbox never started"),
                    EventLoopReport::default(),
                )
            }
            Stage::Paused(paused) => {
                let super::Paused { report, devices } = *paused;
                (report, devices)
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
        (report, devices, retired)
    }
}

fn never_ran(reason: &'static str) -> RunReport {
    RunReport {
        bus: None,
        mmio: None,
        result: Err(MachineError::invalid(Phase::Run, reason)),
        vcpu: None,
    }
}
