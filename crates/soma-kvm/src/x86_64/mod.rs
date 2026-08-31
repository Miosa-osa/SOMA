//! The `x86_64` machine: one KVM VM, one memory slot, one protected-mode vCPU, bounded devices.
//!
//! This module owns the machine-contract floor on a real `/dev/kvm`: memory registration, the
//! in-kernel interrupt controller, bootstrap register state, port I/O dispatch to a diagnostic
//! 16550 model, MMIO dispatch to the five fixed virtio transports, queue-notify ioeventfds,
//! per-slot irqfds, one bounded device thread, deadline enforcement, and ordered cleanup. It
//! proves a raw halt guest, a PVH cold boot to a challenge-bound serial sentinel, and the
//! test-only sandbox machine that boots a compiled Generation for an authenticated guest
//! agent. It makes no readiness, snapshot, isolation, or latency claim on its own.

mod boot_info;
mod channel;
mod cmdline;
mod console_tap;
mod cpuid;
mod devices;
mod elf;
mod error;
mod event_loop;
mod events;
mod guest;
mod halt;
mod kernel;
mod kick;
mod launch_page;
mod layout;
mod loader;
mod memory;
mod mmio;
mod ports;
mod run;
mod sandbox;
mod serial;
mod snapshot;
mod timing;
mod vcpu;
mod watchdog;

use kvm_bindings::{KVM_PIT_SPEAKER_DUMMY, kvm_pit_config};
use kvm_ioctls::{Kvm, VcpuFd, VmFd};

pub use self::{
    channel::{ChannelError, ControlChannel},
    cmdline::BootNonce,
    devices::{BLOCK_SIZE, DeviceIdentity, SandboxDisks},
    elf::ElfError,
    error::{MachineError, MachineErrorKind, Phase},
    event_loop::{EventLoopReport, SlotActivity},
    guest::EXPECTED_SERIAL,
    halt::{HaltGuestConfig, HaltGuestEvidence, InterruptController, run_halt_guest},
    kernel::{BootKernelConfig, KernelBootEvidence, KernelBootFailure, run_kernel_boot},
    launch_page::{LAUNCH_PAGE_GPA, LAUNCH_PAGE_SIZE, LAUNCH_PAGE_SLOT},
    memory::SharedRam,
    mmio::MmioCounters,
    ports::BusCounters,
    run::GuestExit,
    sandbox::{
        Milestone, MilestoneMark, NetworkAttachment, SandboxConfig, SandboxEvidence, SandboxMachine,
    },
    serial::SerialCounters,
    snapshot::{
        Artifact, ArtifactEvidence, CaptureExpectation, CaptureOutcome, CaptureRequest,
        RestoreFacts, RestoreRequest, Restored, SnapshotError, SnapshotPaths, Sterile,
        SterileRequest, VerifiedCapture, capture, inspect_capture_artifacts, restore,
        restore_sterile,
    },
    timing::PhaseTiming,
};
use self::{layout::GuestLayout, memory::GuestRam, timing::Stopwatch};

/// Owned KVM resources in the order they must be released: VM, then KVM, then guest memory.
struct Machine {
    vm: VmFd,
    kvm: Kvm,
    ram: GuestRam,
}

impl Machine {
    /// Opens KVM, probes the capability contract, creates the VM, and registers guest RAM.
    fn create(ram_bytes: u64, clock: &mut Stopwatch) -> Result<Self, MachineError> {
        let kvm = Kvm::new().map_err(|error| MachineError::os(Phase::Open, error))?;
        clock.lap(Phase::Open);
        super::probe()
            .map_err(|error| MachineError::new(Phase::Probe, MachineErrorKind::Probe(error)))?;
        clock.lap(Phase::Probe);
        let layout = GuestLayout::new(ram_bytes)?;
        let vm = kvm
            .create_vm()
            .map_err(|error| MachineError::os(Phase::CreateVm, error))?;
        clock.lap(Phase::CreateVm);
        let ram = GuestRam::map(layout)?;
        clock.lap(Phase::MapMemory);
        ram.register(&vm)?;
        clock.lap(Phase::RegisterMemory);
        Ok(Self { vm, kvm, ram })
    }

    /// Takes ownership of resources another constructor produced, in release order.
    const fn adopt(kvm: Kvm, vm: VmFd, ram: GuestRam) -> Self {
        Self { vm, kvm, ram }
    }

    const fn vm_fd(&self) -> &VmFd {
        &self.vm
    }

    const fn kvm_fd(&self) -> &Kvm {
        &self.kvm
    }

    fn shared_ram(&self) -> memory::SharedRam {
        self.ram.shared()
    }

    /// Registers guest RAM at exactly the layout the snapshot certified.
    ///
    /// Version 1 certifies one slot covering all of guest RAM from address zero; anything
    /// else is a machine this implementation cannot reproduce.
    fn register_certified_slots(
        &self,
        state: &crate::snapshot::kvm_state::VmState,
    ) -> Result<(), MachineError> {
        let certified = state.slots();
        let expected = self.ram.layout().ram_bytes();
        let single = certified.first().is_some_and(|slot| {
            slot.slot == 0
                && slot.guest_address == 0
                && slot.memory_offset == 0
                && slot.size == expected
        });
        if certified.len() != 1 || !single {
            return Err(MachineError::invalid(
                Phase::Restore,
                "the certified memory-slot layout is not the version 1 single-slot layout",
            ));
        }
        self.ram.register(&self.vm)
    }

    /// Recreates the TSS window, the in-kernel interrupt controller, the timer, and the
    /// SOMA-owned interrupt routes, in the order KVM requires before any vCPU exists.
    fn recreate_platform(
        &self,
        state: &crate::snapshot::kvm_state::VmState,
        routing: &crate::snapshot::kvm_state::IrqRoutingState,
    ) -> Result<(), snapshot::SnapshotError> {
        let tss = usize::try_from(state.tss_address())
            .map_err(|_| MachineError::invalid(Phase::Restore, "TSS address overflow"))?;
        self.vm
            .set_tss_address(tss)
            .map_err(|error| MachineError::os(Phase::Restore, error))?;
        self.vm
            .create_irq_chip()
            .map_err(|error| MachineError::os(Phase::Restore, error))?;
        let config = kvm_pit_config {
            flags: KVM_PIT_SPEAKER_DUMMY,
            ..kvm_pit_config::default()
        };
        self.vm
            .create_pit2(config)
            .map_err(|error| MachineError::os(Phase::Restore, error))?;
        snapshot::write_routing(&self.vm, routing)
    }

    /// Sets the TSS window and optionally creates the in-kernel interrupt controller and PIT.
    fn configure_platform(
        &self,
        interrupt_controller: InterruptController,
        pit: bool,
        clock: &mut Stopwatch,
    ) -> Result<(), MachineError> {
        let tss = usize::try_from(layout::TSS_ADDRESS)
            .map_err(|_| MachineError::invalid(Phase::TssAddress, "TSS address overflow"))?;
        self.vm
            .set_tss_address(tss)
            .map_err(|error| MachineError::os(Phase::TssAddress, error))?;
        clock.lap(Phase::TssAddress);
        if interrupt_controller == InterruptController::InKernel {
            self.vm
                .create_irq_chip()
                .map_err(|error| MachineError::os(Phase::IrqChip, error))?;
            clock.lap(Phase::IrqChip);
        }
        if pit {
            if interrupt_controller != InterruptController::InKernel {
                return Err(MachineError::invalid(
                    Phase::Pit,
                    "the in-kernel PIT requires the in-kernel interrupt controller",
                ));
            }
            let config = kvm_pit_config {
                flags: KVM_PIT_SPEAKER_DUMMY,
                ..kvm_pit_config::default()
            };
            self.vm
                .create_pit2(config)
                .map_err(|error| MachineError::os(Phase::Pit, error))?;
            clock.lap(Phase::Pit);
        }
        Ok(())
    }

    /// Creates vCPU 0 with the filtered CPUID template and the contract's protected-mode state.
    fn boot_vcpu(&self, entry: u64, clock: &mut Stopwatch) -> Result<VcpuFd, MachineError> {
        let vcpu = self
            .vm
            .create_vcpu(0)
            .map_err(|error| MachineError::os(Phase::CreateVcpu, error))?;
        clock.lap(Phase::CreateVcpu);
        cpuid::install(&self.kvm, &vcpu)?;
        clock.lap(Phase::Cpuid);
        vcpu::install_registers(&vcpu, entry)?;
        clock.lap(Phase::Regs);
        Ok(vcpu)
    }
}
