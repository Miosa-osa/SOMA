//! Modern virtio-mmio transport and split-virtqueue implementation.
//!
//! This is pure, target-independent Rust with no KVM calls and no `unsafe`.
//! It is a transport and queue implementation with tests, not a working
//! device, bus, or sandbox.

pub mod device;
pub mod devices;
pub mod guest_memory;
pub mod queue;
pub mod transport;

pub use device::{
    ActivateError, ConfigAccessError, DeviceStateError, MAX_CONFIG_LEN, MAX_QUEUES, SOMA_VENDOR_ID,
    VIRTIO_F_VERSION_1, VirtioDevice,
};
#[cfg(unix)]
pub use devices::block::backend::FileBackend;
pub use devices::block::backend::{BackendError as BlockBackendError, BlockBackend, MemoryBackend};
pub use devices::block::request::{
    BLK_ID_LEN, BlockOp, BlockRequest, MAX_REQUEST_BYTES, REQUEST_HEADER_LEN, RequestError,
    RequestLimits, SECTOR_SIZE, VIRTIO_BLK_S_IOERR, VIRTIO_BLK_S_OK, VIRTIO_BLK_S_UNSUPP,
    VIRTIO_BLK_T_FLUSH, VIRTIO_BLK_T_GET_ID, VIRTIO_BLK_T_IN, VIRTIO_BLK_T_OUT, parse_request,
};
pub use devices::block::state::{BLOCK_STATE_LEN, BLOCK_STATE_VERSION, BlockState};
pub use devices::block::{
    BLOCK_CONFIG_LEN, BLOCK_QUEUE_MAX, BLOCK_SERIAL_LEN, BlockConfigError, BlockCounters,
    BlockDevice, BlockRole, VIRTIO_BLK_DEVICE_ID, VIRTIO_BLK_F_BLK_SIZE, VIRTIO_BLK_F_FLUSH,
    VIRTIO_BLK_F_RO,
};
pub use devices::segments::{read_readable, write_writable};
pub use devices::service::{ChainHandler, DeviceFault, ServiceError, ServiceReport, service_queue};
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
