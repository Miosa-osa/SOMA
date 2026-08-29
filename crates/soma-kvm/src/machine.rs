use std::{error::Error, fmt};

use kvm_ioctls::{Kvm, VcpuFd, VmFd};

/// One owned KVM machine and its vCPUs.
///
/// This is the resource-ownership foundation for the SOMA VMM.
/// It deliberately does not claim guest boot, device emulation, or command readiness yet.
pub struct KvmMachine {
    vm: VmFd,
    vcpus: Vec<VcpuFd>,
}

#[derive(Debug)]
pub enum KvmMachineError {
    Kvm(kvm_ioctls::Error),
    InvalidVcpuCount,
}

impl fmt::Display for KvmMachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kvm(error) => write!(formatter, "KVM machine operation failed: {error}"),
            Self::InvalidVcpuCount => formatter.write_str("vCPU count must be non-zero"),
        }
    }
}

impl Error for KvmMachineError {}

impl From<kvm_ioctls::Error> for KvmMachineError {
    fn from(error: kvm_ioctls::Error) -> Self {
        Self::Kvm(error)
    }
}

impl KvmMachine {
    /// Creates one KVM VM and exactly `vcpu_count` vCPU file descriptors.
    ///
    /// # Errors
    ///
    /// Returns an error when KVM is unavailable or any VM/vCPU descriptor cannot be created.
    pub fn create(vcpu_count: u16) -> Result<Self, KvmMachineError> {
        if vcpu_count == 0 {
            return Err(KvmMachineError::InvalidVcpuCount);
        }
        let kvm = Kvm::new()?;
        let vm = kvm.create_vm()?;
        let mut vcpus = Vec::with_capacity(usize::from(vcpu_count));
        for index in 0..vcpu_count {
            vcpus.push(vm.create_vcpu(index.into())?);
        }
        Ok(Self { vm, vcpus })
    }

    /// Returns the number of owned vCPUs.
    #[must_use]
    pub fn vcpu_count(&self) -> usize {
        self.vcpus.len()
    }

    /// Returns a reference to the underlying VM descriptor for device setup.
    #[must_use]
    pub const fn vm(&self) -> &VmFd {
        &self.vm
    }
}
