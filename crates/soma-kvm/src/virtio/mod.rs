//! Modern virtio-mmio transport and split-virtqueue implementation.
//!
//! This is pure, target-independent Rust with no KVM calls and no `unsafe`.
//! It is a transport and queue implementation with tests, not a working
//! device, bus, or sandbox.

pub mod bus;
pub mod device;
pub mod devices;
pub mod guest_memory;
pub mod queue;
pub mod transport;

pub use bus::slots::{PendingWork, SlotRestoreError, SlotSnapshot};
pub use bus::{
    BusConfigError, BusDevices, BusEvent, BusViolation, DeviceSet, FIRST_GSI, IrqSink,
    MMIO_WINDOW_BASE, MmioBus, NotifySource, SLOT_COUNT, Slot, kernel_command_line,
};
pub use device::{
    ActivateError, ConfigAccessError, DeviceStateError, MAX_CONFIG_LEN, MAX_QUEUES, SOMA_VENDOR_ID,
    VIRTIO_F_VERSION_1, VirtioDevice,
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use devices::block::backend::Detached;
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
pub use devices::net::backend::{
    LOOPBACK_QUEUE_LIMIT, LoopbackBackend, LoopbackHandle, NetBackend, NetBackendError, TapBackend,
};
pub use devices::net::frame::{
    FrameError, MAX_FRAME_LEN, MIN_FRAME_LEN, VIRTIO_NET_HDR_LEN, validate_tx,
};
pub use devices::net::rx::deliver_rx;
pub use devices::net::state::{NET_STATE_LEN, NET_STATE_VERSION, NetState};
pub use devices::net::{
    MAX_RX_CHAIN_BYTES, NET_CONFIG_LEN, NET_FEATURES, NET_QUEUE_MAX, NET_RX_QUEUE, NET_TX_QUEUE,
    NetCounters, NetDevice, VIRTIO_NET_DEVICE_ID, VIRTIO_NET_F_MAC,
};
#[cfg(unix)]
pub use devices::rng::backend::OsEntropy;
pub use devices::rng::backend::{EntropyBackend, EntropyError};
pub use devices::rng::state::{RNG_STATE_LEN, RNG_STATE_VERSION, RngState};
pub use devices::rng::{
    MAX_ENTROPY_REQUEST, RNG_FEATURES, RNG_QUEUE_MAX, RngCounters, RngDevice, VIRTIO_RNG_DEVICE_ID,
};
pub use devices::segments::{read_readable, write_writable};
pub use devices::service::{ChainHandler, DeviceFault, ServiceError, ServiceReport, service_queue};
pub use devices::vsock::connection::{HOST_TX_BUFFER, HostEndpoint};
pub use devices::vsock::credit::{Credit, CreditError, HOST_BUF_ALLOC};
pub use devices::vsock::packet::{
    HOST_CID, MAX_PAYLOAD_LEN, PacketError, SOMA_CONTROL_PORT, VIRTIO_VSOCK_DEVICE_ID,
    VSOCK_EVENT_TRANSPORT_RESET, VSOCK_HDR_LEN, VSOCK_OP_CREDIT_REQUEST, VSOCK_OP_CREDIT_UPDATE,
    VSOCK_OP_INVALID, VSOCK_OP_REQUEST, VSOCK_OP_RESPONSE, VSOCK_OP_RST, VSOCK_OP_RW,
    VSOCK_OP_SHUTDOWN, VSOCK_SHUTDOWN_RCV, VSOCK_SHUTDOWN_SEND, VSOCK_TYPE_STREAM, VsockHeader,
    parse_tx,
};
pub use devices::vsock::rx::{deliver_events, deliver_rx as deliver_vsock_rx};
pub use devices::vsock::state::{VSOCK_STATE_LEN, VSOCK_STATE_VERSION, VsockState};
pub use devices::vsock::{
    CID_ANY, MAX_OUTBOUND_PACKETS, MAX_PENDING_EVENTS, MIN_GUEST_CID, VSOCK_CONFIG_LEN,
    VSOCK_EVENT_QUEUE, VSOCK_FEATURES, VSOCK_QUEUE_MAX, VSOCK_RX_QUEUE, VSOCK_TX_QUEUE,
    VsockConfigError, VsockCounters, VsockDevice,
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
