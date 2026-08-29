//! Modern virtio-mmio transport and split-virtqueue implementation.
//!
//! This is pure, target-independent Rust with no KVM calls and no `unsafe`.
//! It is a transport and queue implementation with tests, not a working
//! device, bus, or sandbox.

pub mod device;
pub mod guest_memory;
pub mod queue;
pub mod transport;

pub use device::{
    ActivateError, ConfigAccessError, DeviceStateError, MAX_CONFIG_LEN, MAX_QUEUES, SOMA_VENDOR_ID,
    VIRTIO_F_VERSION_1, VirtioDevice,
};
pub use guest_memory::{
    GuestAddress, GuestMemory, GuestMemoryError, GuestValue, RegionLayoutError, VecGuestMemory,
};
pub use queue::chain::{
    ChainLimits, ChainSegment, ChainViolation, DESCRIPTOR_SIZE, Descriptor, DescriptorChain,
    MAX_QUEUE_SIZE, VIRTQ_DESC_F_INDIRECT, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE, walk_chain,
};
pub use queue::layout::{LayoutViolation, QueueLayout, validate_size};
pub use queue::state::{QUEUE_STATE_LEN, QueueState, QueueStateError};
pub use queue::violation::{QueueViolation, QueueViolationCounters, QueueViolationKind};
pub use queue::{Queue, VIRTQ_AVAIL_F_NO_INTERRUPT};
pub use transport::registers::{AccessWidth, MMIO_PAGE_SIZE, Register};
pub use transport::state::{
    RestoreError, TRANSPORT_STATE_HEADER_LEN, TransportState, TransportStateError,
};
pub use transport::status::{DeviceStatus, StatusViolation, StatusWrite};
pub use transport::violation::{
    TransportViolation, TransportViolationCounters, TransportViolationKind,
};
pub use transport::{
    INTERRUPT_CONFIG_CHANGE, INTERRUPT_USED_BUFFER, MmioTransport, TransportConfigError,
    TransportEvent,
};
