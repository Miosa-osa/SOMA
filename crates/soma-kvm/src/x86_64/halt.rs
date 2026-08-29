//! The raw halt-guest proof: the floor test that the machine still passes under every change.

use std::time::Duration;

use super::{
    Machine, MachineError, Phase, guest,
    ports::PortBus,
    run::GuestExit,
    serial::Serial,
    timing::{PhaseTiming, Stopwatch},
    watchdog,
};

/// Whether the VM receives KVM's in-kernel interrupt controller.
///
/// With the in-kernel controller, `hlt` parks the vCPU inside KVM waiting for an interrupt that
/// never arrives, so the halt proof can only end through the watchdog. Without it, KVM reports
/// `KVM_EXIT_HLT` to userspace and the proof observes the halt directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptController {
    /// No interrupt controller; `hlt` exits to userspace.
    None,
    /// `KVM_CREATE_IRQCHIP`; `hlt` blocks in the kernel.
    InKernel,
}

/// Inputs for one halt-guest proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaltGuestConfig {
    ram_bytes: u64,
    timeout: Duration,
    interrupt_controller: InterruptController,
}

impl HaltGuestConfig {
    /// Creates a configuration with the given guest RAM size and run deadline.
    #[must_use]
    pub const fn new(ram_bytes: u64, timeout: Duration) -> Self {
        Self {
            ram_bytes,
            timeout,
            interrupt_controller: InterruptController::None,
        }
    }

    /// Selects the interrupt-controller mode.
    #[must_use]
    pub const fn with_interrupt_controller(mut self, mode: InterruptController) -> Self {
        self.interrupt_controller = mode;
        self
    }
}

/// Retained evidence from a successful proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaltGuestEvidence {
    serial: Vec<u8>,
    exit: GuestExit,
    timings: Vec<PhaseTiming>,
    total_ns: u64,
}

impl HaltGuestEvidence {
    /// Every byte the guest wrote to the serial port, in order.
    #[must_use]
    pub fn serial(&self) -> &[u8] {
        &self.serial
    }

    /// How the guest stopped.
    #[must_use]
    pub const fn exit(&self) -> GuestExit {
        self.exit
    }

    /// Per-phase durations in lifecycle order, ending with cleanup.
    #[must_use]
    pub fn timings(&self) -> &[PhaseTiming] {
        &self.timings
    }

    /// Nanoseconds from the first KVM open through completed cleanup.
    #[must_use]
    pub const fn total_ns(&self) -> u64 {
        self.total_ns
    }
}

/// Creates one KVM VM, runs the halt guest on one vCPU, and releases everything it owned.
///
/// # Errors
///
/// Returns a typed error naming the failed phase when the host lacks KVM or a required
/// capability, when the configuration violates the machine contract, when the guest exits in an
/// unexpected way, or when it does not halt before the deadline.
pub fn run_halt_guest(config: &HaltGuestConfig) -> Result<HaltGuestEvidence, MachineError> {
    let mut clock = Stopwatch::new();
    let mut machine = Machine::create(config.ram_bytes, &mut clock)?;
    let outcome = prepare_and_run(&mut machine, config, &mut clock);
    drop(machine);
    clock.lap(Phase::Cleanup);
    let (serial, exit) = outcome?;
    let (total_ns, timings) = clock.finish();
    Ok(HaltGuestEvidence {
        serial,
        exit,
        timings,
        total_ns,
    })
}

fn prepare_and_run(
    machine: &mut Machine,
    config: &HaltGuestConfig,
    clock: &mut Stopwatch,
) -> Result<(Vec<u8>, GuestExit), MachineError> {
    machine.configure_platform(config.interrupt_controller, false, clock)?;
    let entry = guest::load(&mut machine.ram)?;
    clock.lap(Phase::LoadGuest);
    let vcpu = machine.boot_vcpu(entry, clock)?;
    let bus = PortBus::new(Serial::new(None));
    let report = watchdog::run_with_deadline(vcpu, bus, None, None, config.timeout);
    clock.lap(Phase::Run);
    let serial = report
        .bus
        .map(|bus| bus.into_serial().into_output())
        .unwrap_or_default();
    let exit = report.result?;
    if exit != GuestExit::Halt {
        return Err(MachineError::invalid(
            Phase::Run,
            "the halt guest stopped without executing hlt",
        ));
    }
    Ok((serial, exit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x86_64::layout;

    #[test]
    fn config_defaults_to_no_interrupt_controller() {
        let config = HaltGuestConfig::new(layout::MIN_RAM_BYTES, Duration::from_secs(1));
        assert_eq!(config.interrupt_controller, InterruptController::None);
        let config = config.with_interrupt_controller(InterruptController::InKernel);
        assert_eq!(config.interrupt_controller, InterruptController::InKernel);
    }

    #[test]
    fn rejects_invalid_ram_before_touching_kvm() {
        let error = run_halt_guest(&HaltGuestConfig::new(4096, Duration::from_secs(1)));
        match error {
            Err(error) if error.phase() == Phase::MapMemory => {}
            Err(error) if error.phase() == Phase::Open || error.phase() == Phase::Probe => {}
            other => panic!("unexpected result {other:?}"),
        }
    }
}
