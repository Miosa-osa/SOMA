#![deny(warnings)]
#![deny(unsafe_code)]

#[cfg(all(test, target_os = "linux", target_arch = "aarch64"))]
mod arm64;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[allow(unsafe_code)]
mod machine;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use linux::{
    KVM_API_VERSION, KvmCapability, KvmProbe, KvmProbeError, KvmProbeOperation, probe,
};

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use machine::{KvmMachine, KvmMachineError};

/// Whether this build target can run SOMA's initial KVM capability probe.
pub const SUPPORTED_TARGET: bool = cfg!(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
));
