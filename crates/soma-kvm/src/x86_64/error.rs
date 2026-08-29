use std::{error::Error, fmt};

use super::super::KvmProbeError;

/// One lifecycle phase of the `x86_64` halt-guest proof.
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
    LoadGuest,
    CreateVcpu,
    Cpuid,
    Sregs,
    Regs,
    Run,
    Join,
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
            Self::LoadGuest => "load guest program and boot structures",
            Self::CreateVcpu => "create vCPU 0",
            Self::Cpuid => "install CPUID",
            Self::Sregs => "install special registers",
            Self::Regs => "install general registers",
            Self::Run => "run vCPU 0",
            Self::Join => "join vCPU thread",
            Self::Cleanup => "release owned resources",
        };
        formatter.write_str(name)
    }
}

/// The reason a phase failed, with guest and host details redacted to stable classifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HaltGuestErrorKind {
    /// A KVM ioctl or system call failed with the given `errno`.
    Os(i32),
    /// The host does not satisfy the KVM capability contract.
    Probe(KvmProbeError),
    /// A validation rule rejected the request or a computed layout.
    Invalid(&'static str),
    /// The guest produced an exit the proof does not accept.
    UnexpectedExit(String),
    /// The guest did not halt before the deadline and was interrupted by the watchdog.
    Timeout,
    /// The vCPU thread panicked or disconnected without a result.
    WorkerLost,
}

impl fmt::Display for HaltGuestErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Os(errno) => write!(formatter, "operating system error errno {errno}"),
            Self::Probe(error) => write!(formatter, "{error}"),
            Self::Invalid(reason) => formatter.write_str(reason),
            Self::UnexpectedExit(exit) => write!(formatter, "unexpected vCPU exit {exit}"),
            Self::Timeout => formatter.write_str("guest did not halt before the deadline"),
            Self::WorkerLost => formatter.write_str("vCPU thread ended without a result"),
        }
    }
}

/// A typed failure of the `x86_64` halt-guest proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaltGuestError {
    phase: Phase,
    kind: HaltGuestErrorKind,
}

impl HaltGuestError {
    pub(super) const fn new(phase: Phase, kind: HaltGuestErrorKind) -> Self {
        Self { phase, kind }
    }

    pub(super) fn os(phase: Phase, error: kvm_ioctls::Error) -> Self {
        Self::new(phase, HaltGuestErrorKind::Os(error.errno()))
    }

    pub(super) fn last_os(phase: Phase) -> Self {
        Self::new(
            phase,
            HaltGuestErrorKind::Os(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)),
        )
    }

    pub(super) const fn invalid(phase: Phase, reason: &'static str) -> Self {
        Self::new(phase, HaltGuestErrorKind::Invalid(reason))
    }

    /// Returns the lifecycle phase that failed.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Returns the failure classification.
    #[must_use]
    pub const fn kind(&self) -> &HaltGuestErrorKind {
        &self.kind
    }
}

impl fmt::Display for HaltGuestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.phase, self.kind)
    }
}

impl Error for HaltGuestError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_phase_and_kind() {
        let error = HaltGuestError::new(Phase::Run, HaltGuestErrorKind::Timeout);
        assert_eq!(
            error.to_string(),
            "run vCPU 0: guest did not halt before the deadline"
        );
        assert_eq!(error.phase(), Phase::Run);
        assert_eq!(error.kind(), &HaltGuestErrorKind::Timeout);
    }
}
