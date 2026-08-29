use std::{error::Error, fmt};

use kvm_ioctls::{Cap, Kvm};

use crate::linux::KVM_API_VERSION;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootCapability {
    UserMemory,
    OneReg,
    DeviceCtrl,
    ArmPsci02,
}

impl BootCapability {
    const fn rust_vmm(self) -> Cap {
        match self {
            Self::UserMemory => Cap::UserMemory,
            Self::OneReg => Cap::OneReg,
            Self::DeviceCtrl => Cap::DeviceCtrl,
            Self::ArmPsci02 => Cap::ArmPsci02,
        }
    }
}

const REQUIRED_CAPABILITIES: &[BootCapability] = &[
    BootCapability::UserMemory,
    BootCapability::OneReg,
    BootCapability::DeviceCtrl,
    BootCapability::ArmPsci02,
];

#[derive(Debug)]
pub(crate) enum BootHostError {
    UnexpectedApiVersion { actual: i32 },
    MissingCapability(BootCapability),
    ReadVcpuMmapSize(kvm_ioctls::Error),
    InvalidVcpuMmapSize,
}

impl fmt::Display for BootHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedApiVersion { actual } => write!(
                formatter,
                "KVM interface version mismatch: expected {KVM_API_VERSION}, got {actual}"
            ),
            Self::MissingCapability(capability) => {
                write!(
                    formatter,
                    "required KVM capability is missing: {capability:?}"
                )
            }
            Self::ReadVcpuMmapSize(error) => {
                write!(formatter, "read vCPU mapping size: {error}")
            }
            Self::InvalidVcpuMmapSize => formatter.write_str("KVM returned zero vCPU mapping size"),
        }
    }
}

impl Error for BootHostError {}

pub(crate) fn validate(kvm: &Kvm) -> Result<(), BootHostError> {
    let api_version = kvm.get_api_version();
    if api_version != KVM_API_VERSION {
        return Err(BootHostError::UnexpectedApiVersion {
            actual: api_version,
        });
    }
    for &capability in REQUIRED_CAPABILITIES {
        if !kvm.check_extension(capability.rust_vmm()) {
            return Err(BootHostError::MissingCapability(capability));
        }
    }
    let mmap_size = kvm
        .get_vcpu_mmap_size()
        .map_err(BootHostError::ReadVcpuMmapSize)?;
    if mmap_size == 0 {
        return Err(BootHostError::InvalidVcpuMmapSize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_boot_capability_policy_matches_its_actual_ioctls() {
        assert_eq!(
            REQUIRED_CAPABILITIES,
            &[
                BootCapability::UserMemory,
                BootCapability::OneReg,
                BootCapability::DeviceCtrl,
                BootCapability::ArmPsci02,
            ]
        );
    }
}
