//! The `x86_64` machine floor: one KVM VM, one memory slot, one protected-mode vCPU, one halt.
//!
//! This module proves the bottom of the `x86_64` machine contract on a real `/dev/kvm`: memory
//! registration, bootstrap register state, port I/O exit capture, `hlt`, deadline enforcement,
//! and ordered cleanup. It boots no kernel, emulates no device beyond one output port, and
//! makes no sandbox, readiness, isolation, or latency claim.

mod boot_info;
mod error;
mod guest;
mod kick;
mod layout;
mod memory;
mod run;
mod vcpu;
mod watchdog;

use std::time::{Duration, Instant};

use kvm_ioctls::{Kvm, VmFd};

pub use self::{
    error::{HaltGuestError, HaltGuestErrorKind, Phase},
    guest::EXPECTED_SERIAL,
    run::GuestExit,
};
use self::{layout::GuestLayout, memory::GuestRam};

/// Whether the VM receives KVM's in-kernel interrupt controller.
///
/// With the in-kernel controller, `hlt` parks the vCPU inside KVM waiting for an interrupt that
/// never arrives, so the proof can only end through the watchdog. Without it, KVM reports
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

/// Monotonic duration of one completed phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseTiming {
    phase: Phase,
    elapsed_ns: u64,
}

impl PhaseTiming {
    /// The phase that completed.
    #[must_use]
    pub const fn phase(self) -> Phase {
        self.phase
    }

    /// Nanoseconds spent in that phase alone.
    #[must_use]
    pub const fn elapsed_ns(self) -> u64 {
        self.elapsed_ns
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

struct Stopwatch {
    started: Instant,
    last: Instant,
    timings: Vec<PhaseTiming>,
}

impl Stopwatch {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            timings: Vec::new(),
        }
    }

    fn lap(&mut self, phase: Phase) {
        let now = Instant::now();
        self.timings.push(PhaseTiming {
            phase,
            elapsed_ns: saturating_ns(now.duration_since(self.last)),
        });
        self.last = now;
    }
}

/// Owned KVM resources in the order they must be released: VM, then KVM, then guest memory.
struct Machine {
    vm: VmFd,
    kvm: Kvm,
    ram: GuestRam,
}

/// Creates one KVM VM, runs the halt guest on one vCPU, and releases everything it owned.
///
/// # Errors
///
/// Returns a typed error naming the failed phase when the host lacks KVM or a required
/// capability, when the configuration violates the machine contract, when the guest exits in an
/// unexpected way, or when it does not halt before the deadline.
pub fn run_halt_guest(config: &HaltGuestConfig) -> Result<HaltGuestEvidence, HaltGuestError> {
    let mut clock = Stopwatch::new();
    let kvm = Kvm::new().map_err(|error| HaltGuestError::os(Phase::Open, error))?;
    clock.lap(Phase::Open);
    super::probe()
        .map_err(|error| HaltGuestError::new(Phase::Probe, HaltGuestErrorKind::Probe(error)))?;
    clock.lap(Phase::Probe);
    let layout = GuestLayout::new(config.ram_bytes)?;
    let vm = kvm
        .create_vm()
        .map_err(|error| HaltGuestError::os(Phase::CreateVm, error))?;
    clock.lap(Phase::CreateVm);
    let ram = GuestRam::map(layout)?;
    clock.lap(Phase::MapMemory);
    ram.register(&vm)?;
    clock.lap(Phase::RegisterMemory);
    let mut machine = Machine { vm, kvm, ram };
    let outcome = prepare_and_run(&mut machine, config, &mut clock);
    drop(machine);
    clock.lap(Phase::Cleanup);
    let outcome = outcome?;
    Ok(HaltGuestEvidence {
        serial: outcome.serial,
        exit: outcome.exit,
        total_ns: saturating_ns(clock.last.duration_since(clock.started)),
        timings: clock.timings,
    })
}

fn prepare_and_run(
    machine: &mut Machine,
    config: &HaltGuestConfig,
    clock: &mut Stopwatch,
) -> Result<run::RunOutcome, HaltGuestError> {
    let tss = usize::try_from(layout::TSS_ADDRESS)
        .map_err(|_| HaltGuestError::invalid(Phase::TssAddress, "TSS address overflow"))?;
    machine
        .vm
        .set_tss_address(tss)
        .map_err(|error| HaltGuestError::os(Phase::TssAddress, error))?;
    clock.lap(Phase::TssAddress);
    if config.interrupt_controller == InterruptController::InKernel {
        machine
            .vm
            .create_irq_chip()
            .map_err(|error| HaltGuestError::os(Phase::IrqChip, error))?;
        clock.lap(Phase::IrqChip);
    }
    let entry = guest::load(&mut machine.ram)?;
    clock.lap(Phase::LoadGuest);
    let vcpu = machine
        .vm
        .create_vcpu(0)
        .map_err(|error| HaltGuestError::os(Phase::CreateVcpu, error))?;
    clock.lap(Phase::CreateVcpu);
    vcpu::install_cpuid(&machine.kvm, &vcpu)?;
    clock.lap(Phase::Cpuid);
    vcpu::install_registers(&vcpu, entry)?;
    clock.lap(Phase::Regs);
    let result = watchdog::run_with_deadline(vcpu, config.timeout);
    clock.lap(Phase::Run);
    result
}

fn saturating_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_no_interrupt_controller() {
        let config = HaltGuestConfig::new(layout::MIN_RAM_BYTES, Duration::from_secs(1));
        assert_eq!(config.interrupt_controller, InterruptController::None);
        let config = config.with_interrupt_controller(InterruptController::InKernel);
        assert_eq!(config.interrupt_controller, InterruptController::InKernel);
    }

    #[test]
    fn stopwatch_records_phases_in_order() {
        let mut clock = Stopwatch::new();
        clock.lap(Phase::Open);
        clock.lap(Phase::Probe);
        let phases: Vec<Phase> = clock.timings.iter().map(|timing| timing.phase()).collect();
        assert_eq!(phases, [Phase::Open, Phase::Probe]);
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
