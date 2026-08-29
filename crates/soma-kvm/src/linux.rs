use std::{error::Error, fmt};

use kvm_ioctls::{Cap, Kvm};

pub const KVM_API_VERSION: i32 = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvmProbe {
    api_version: i32,
    vcpu_mmap_size: usize,
}

impl KvmProbe {
    #[must_use]
    pub const fn api_version(self) -> i32 {
        self.api_version
    }

    #[must_use]
    pub const fn vcpu_mmap_size(self) -> usize {
        self.vcpu_mmap_size
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvmCapability {
    UserMemory,
    IrqChip,
    IrqFd,
    IoEventFd,
    ImmediateExit,
}

impl KvmCapability {
    const fn rust_vmm_cap(self) -> Cap {
        match self {
            Self::UserMemory => Cap::UserMemory,
            Self::IrqChip => Cap::Irqchip,
            Self::IrqFd => Cap::Irqfd,
            Self::IoEventFd => Cap::Ioeventfd,
            Self::ImmediateExit => Cap::ImmediateExit,
        }
    }
}

#[cfg(target_arch = "x86_64")]
const REQUIRED_CAPABILITIES: &[KvmCapability] = &[
    KvmCapability::UserMemory,
    KvmCapability::IrqChip,
    KvmCapability::IrqFd,
    KvmCapability::IoEventFd,
    KvmCapability::ImmediateExit,
];

#[cfg(target_arch = "aarch64")]
const REQUIRED_CAPABILITIES: &[KvmCapability] = &[
    KvmCapability::UserMemory,
    KvmCapability::IrqFd,
    KvmCapability::IoEventFd,
    KvmCapability::ImmediateExit,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvmProbeOperation {
    Open,
    ReadVcpuMmapSize,
    CreateVm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvmProbeError {
    Io {
        operation: KvmProbeOperation,
        errno: i32,
    },
    UnexpectedApiVersion {
        expected: i32,
        actual: i32,
    },
    MissingCapability(KvmCapability),
    InvalidVcpuMmapSize(usize),
}

impl fmt::Display for KvmProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, errno } => {
                write!(formatter, "KVM {operation:?} failed with errno {errno}")
            }
            Self::UnexpectedApiVersion { expected, actual } => {
                write!(
                    formatter,
                    "KVM interface version mismatch: expected {expected}, got {actual}"
                )
            }
            Self::MissingCapability(capability) => {
                write!(
                    formatter,
                    "required KVM capability is missing: {capability:?}"
                )
            }
            Self::InvalidVcpuMmapSize(actual) => {
                write!(
                    formatter,
                    "KVM returned invalid vCPU mapping size: {actual}"
                )
            }
        }
    }
}

impl Error for KvmProbeError {}

/// Opens KVM, verifies the v1 capability contract, and creates an empty VM as a smoke check.
///
/// # Errors
///
/// Returns [`KvmProbeError`] when `/dev/kvm` cannot be opened, the API version or a required
/// capability is incompatible, the vCPU mapping size cannot be read, or an empty VM cannot be
/// created.
pub fn probe() -> Result<KvmProbe, KvmProbeError> {
    let kvm = Kvm::new().map_err(|error| io_error(KvmProbeOperation::Open, error.errno()))?;
    let api_version = kvm.get_api_version();
    if api_version != KVM_API_VERSION {
        return Err(KvmProbeError::UnexpectedApiVersion {
            expected: KVM_API_VERSION,
            actual: api_version,
        });
    }

    for &capability in REQUIRED_CAPABILITIES {
        if !kvm.check_extension(capability.rust_vmm_cap()) {
            return Err(KvmProbeError::MissingCapability(capability));
        }
    }

    let vcpu_mmap_size = kvm
        .get_vcpu_mmap_size()
        .map_err(|error| io_error(KvmProbeOperation::ReadVcpuMmapSize, error.errno()))?;
    validate_vcpu_mmap_size(vcpu_mmap_size)?;
    let vm = kvm
        .create_vm()
        .map_err(|error| io_error(KvmProbeOperation::CreateVm, error.errno()))?;
    drop(vm);

    Ok(KvmProbe {
        api_version,
        vcpu_mmap_size,
    })
}

const fn io_error(operation: KvmProbeOperation, errno: i32) -> KvmProbeError {
    KvmProbeError::Io { operation, errno }
}

const fn validate_vcpu_mmap_size(value: usize) -> Result<(), KvmProbeError> {
    if value == 0 {
        Err(KvmProbeError::InvalidVcpuMmapSize(value))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{KvmCapability, KvmProbeError, REQUIRED_CAPABILITIES, validate_vcpu_mmap_size};

    #[test]
    fn rejects_a_zero_vcpu_mapping_size() {
        assert_eq!(
            validate_vcpu_mmap_size(0),
            Err(KvmProbeError::InvalidVcpuMmapSize(0))
        );
        assert_eq!(validate_vcpu_mmap_size(1), Ok(()));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn x86_64_capability_policy_is_exact() {
        assert_eq!(
            REQUIRED_CAPABILITIES,
            &[
                KvmCapability::UserMemory,
                KvmCapability::IrqChip,
                KvmCapability::IrqFd,
                KvmCapability::IoEventFd,
                KvmCapability::ImmediateExit,
            ]
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn arm64_capability_policy_is_exact() {
        assert_eq!(
            REQUIRED_CAPABILITIES,
            &[
                KvmCapability::UserMemory,
                KvmCapability::IrqFd,
                KvmCapability::IoEventFd,
                KvmCapability::ImmediateExit,
            ]
        );
    }
}
