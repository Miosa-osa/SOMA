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

/// The `x86_64` machine floor: memory slot, protected-mode vCPU, port I/O capture, and `hlt`.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub mod x86_64;

mod virtio;

pub mod snapshot;

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

pub use virtio::{
    AccessWidth, ActivateError, ChainLimits, ChainSegment, ChainViolation, ConfigAccessError,
    DESCRIPTOR_SIZE, Descriptor, DescriptorChain, DeviceStateError, DeviceStatus, GuestAddress,
    GuestMemory, GuestMemoryError, GuestValue, INTERRUPT_CONFIG_CHANGE, INTERRUPT_USED_BUFFER,
    LayoutViolation, MAX_CONFIG_LEN, MAX_QUEUE_SIZE, MAX_QUEUES, MMIO_PAGE_SIZE, MmioTransport,
    QUEUE_STATE_LEN, Queue, QueueLayout, QueueState, QueueStateError, QueueViolation,
    QueueViolationCounters, QueueViolationKind, RegionLayoutError, Register, RestoreError,
    SOMA_VENDOR_ID, StatusViolation, StatusWrite, TRANSPORT_STATE_HEADER_LEN, TransportConfigError,
    TransportEvent, TransportState, TransportStateError, TransportViolation,
    TransportViolationCounters, TransportViolationKind, VIRTIO_F_VERSION_1,
    VIRTQ_AVAIL_F_NO_INTERRUPT, VIRTQ_DESC_F_INDIRECT, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE,
    VecGuestMemory, VirtioDevice, validate_size, walk_chain,
};

/// Whether this build target can run SOMA's initial KVM capability probe.
pub const SUPPORTED_TARGET: bool = cfg!(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
));
