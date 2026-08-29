use std::{error::Error, fmt};

use super::{super::KvmProbeError, elf::ElfError};
use crate::virtio::{BlockConfigError, BusConfigError, EntropyError, VsockConfigError};

/// One lifecycle phase of an `x86_64` machine proof.
///
/// Every failure names the phase that produced it so cleanup evidence can state exactly which
/// owned resources existed when the proof stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Open,
    Probe,
    CreateVm,
    MapMemory,
    RegisterMemory,
    TssAddress,
    IrqChip,
    Pit,
    ReadKernel,
    LoadGuest,
    CreateVcpu,
    Cpuid,
    Sregs,
    Regs,
    Devices,
    LaunchPage,
    Events,
    EventLoop,
    Run,
    Control,
    Join,
    Capture,
    Restore,
    Cleanup,
}

impl fmt::Display for Phase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Open => "open KVM",
            Self::Probe => "probe KVM capabilities",
            Self::CreateVm => "create VM",
            Self::MapMemory => "map guest RAM",
            Self::RegisterMemory => "register guest RAM",
            Self::TssAddress => "set TSS address",
            Self::IrqChip => "create in-kernel interrupt controller",
            Self::Pit => "create in-kernel programmable interval timer",
            Self::ReadKernel => "read kernel and initramfs artifacts",
            Self::LoadGuest => "load guest program and boot structures",
            Self::CreateVcpu => "create vCPU 0",
            Self::Cpuid => "install CPUID",
            Self::Sregs => "install special registers",
            Self::Regs => "install general registers",
            Self::Devices => "build virtio devices and bus",
            Self::LaunchPage => "map launch page slot",
            Self::Events => "register ioeventfds and irqfds",
            Self::EventLoop => "start device event loop",
            Self::Run => "run vCPU 0",
            Self::Control => "guest control session",
            Self::Join => "join vCPU thread",
            Self::Capture => "capture the machine snapshot",
            Self::Restore => "restore the machine snapshot",
            Self::Cleanup => "release owned resources",
        };
        formatter.write_str(name)
    }
}

/// The reason a phase failed, with guest and host details redacted to stable classifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineErrorKind {
    /// A KVM ioctl or system call failed with the given `errno`.
    Os(i32),
    /// The host does not satisfy the KVM capability contract.
    Probe(KvmProbeError),
    /// A validation rule rejected the request or a computed layout.
    Invalid(&'static str),
    /// The kernel image violated the bounded PVH ELF contract.
    Elf(ElfError),
    /// The guest produced an exit the proof does not accept.
    UnexpectedExit(String),
    /// The guest did not stop before the deadline and was interrupted by the watchdog.
    Timeout,
    /// The vCPU thread panicked or disconnected without a result.
    WorkerLost,
    /// The five device models could not be bound to the bus.
    Bus(BusConfigError),
    /// A block device rejected its backend or geometry.
    Block(BlockConfigError),
    /// The vsock device rejected its guest context identifier.
    Vsock(VsockConfigError),
    /// The host entropy source is unavailable.
    Entropy(EntropyError),
    /// The guest touched a guest-physical address outside RAM and the five MMIO pages.
    UnmappedMmio { address: u64 },
    /// The guest issued an MMIO access of a width the transport cannot represent.
    MmioWidth { bytes: usize },
    /// The guest did not overwrite the consumed launch page with zeroes.
    LaunchPageNotErased,
}

impl fmt::Display for MachineErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Os(errno) => write!(formatter, "operating system error errno {errno}"),
            Self::Probe(error) => write!(formatter, "{error}"),
            Self::Invalid(reason) => formatter.write_str(reason),
            Self::Elf(error) => write!(formatter, "kernel ELF rejected: {error}"),
            Self::UnexpectedExit(exit) => write!(formatter, "unexpected vCPU exit {exit}"),
            Self::Timeout => formatter.write_str("guest did not halt before the deadline"),
            Self::WorkerLost => formatter.write_str("vCPU thread ended without a result"),
            Self::Bus(error) => write!(formatter, "{error}"),
            Self::Block(error) => write!(formatter, "{error}"),
            Self::Vsock(error) => write!(formatter, "{error}"),
            Self::Entropy(error) => write!(formatter, "{error}"),
            Self::UnmappedMmio { address } => {
                write!(formatter, "guest accessed unmapped address {address:#x}")
            }
            Self::MmioWidth { bytes } => {
                write!(formatter, "guest issued an MMIO access of {bytes} bytes")
            }
            Self::LaunchPageNotErased => {
                formatter.write_str("the guest consumed the launch page without erasing it")
            }
        }
    }
}

/// A typed failure of an `x86_64` machine proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineError {
    phase: Phase,
    kind: MachineErrorKind,
}

impl MachineError {
    pub(super) const fn new(phase: Phase, kind: MachineErrorKind) -> Self {
        Self { phase, kind }
    }

    pub(super) fn os(phase: Phase, error: kvm_ioctls::Error) -> Self {
        Self::new(phase, MachineErrorKind::Os(error.errno()))
    }

    pub(super) fn io(phase: Phase, error: &std::io::Error) -> Self {
        Self::new(
            phase,
            MachineErrorKind::Os(error.raw_os_error().unwrap_or(0)),
        )
    }

    pub(super) fn last_os(phase: Phase) -> Self {
        Self::io(phase, &std::io::Error::last_os_error())
    }

    pub(super) const fn invalid(phase: Phase, reason: &'static str) -> Self {
        Self::new(phase, MachineErrorKind::Invalid(reason))
    }

    /// Returns the lifecycle phase that failed.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Returns the failure classification.
    #[must_use]
    pub const fn kind(&self) -> &MachineErrorKind {
        &self.kind
    }
}

impl fmt::Display for MachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.phase, self.kind)
    }
}

impl Error for MachineError {}

impl From<ElfError> for MachineError {
    fn from(error: ElfError) -> Self {
        Self::new(Phase::LoadGuest, MachineErrorKind::Elf(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_phase_and_kind() {
        let error = MachineError::new(Phase::Run, MachineErrorKind::Timeout);
        assert_eq!(
            error.to_string(),
            "run vCPU 0: guest did not halt before the deadline"
        );
        assert_eq!(error.phase(), Phase::Run);
        assert_eq!(error.kind(), &MachineErrorKind::Timeout);
    }

    #[test]
    fn elf_errors_land_in_the_load_phase() {
        let error = MachineError::from(ElfError::MissingPvhNote);
        assert_eq!(error.phase(), Phase::LoadGuest);
        assert!(error.to_string().contains("XEN_ELFNOTE_PHYS32_ENTRY"));
    }
}
