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
    AccessWidth, ActivateError, BLK_ID_LEN, BLOCK_CONFIG_LEN, BLOCK_QUEUE_MAX, BLOCK_SERIAL_LEN,
    BLOCK_STATE_LEN, BLOCK_STATE_VERSION, BlockBackend, BlockBackendError, BlockConfigError,
    BlockCounters, BlockDevice, BlockOp, BlockRequest, BlockRole, BlockState, BusConfigError,
    BusDevices, BusEvent, BusViolation, CID_ANY, ChainHandler, ChainLimits, ChainSegment,
    ChainViolation, ConfigAccessError, Credit, CreditError, DESCRIPTOR_SIZE, Descriptor,
    DescriptorChain, DeviceFault, DeviceStateError, DeviceStatus, EntropyBackend, EntropyError,
    FIRST_GSI, FrameError, GuestAddress, GuestMemory, GuestMemoryError, GuestValue, HOST_BUF_ALLOC,
    HOST_CID, HOST_TX_BUFFER, HostEndpoint, INTERRUPT_CONFIG_CHANGE, INTERRUPT_USED_BUFFER,
    IrqSink, LOOPBACK_QUEUE_LIMIT, LayoutViolation, LoopbackBackend, LoopbackHandle,
    MAX_CONFIG_LEN, MAX_ENTROPY_REQUEST, MAX_FRAME_LEN, MAX_OUTBOUND_PACKETS, MAX_PAYLOAD_LEN,
    MAX_PENDING_EVENTS, MAX_QUEUE_SIZE, MAX_QUEUES, MAX_REQUEST_BYTES, MAX_RX_CHAIN_BYTES,
    MIN_FRAME_LEN, MIN_GUEST_CID, MMIO_PAGE_SIZE, MMIO_WINDOW_BASE, MemoryBackend, MmioBus,
    MmioTransport, NET_CONFIG_LEN, NET_FEATURES, NET_QUEUE_MAX, NET_RX_QUEUE, NET_STATE_LEN,
    NET_STATE_VERSION, NET_TX_QUEUE, NetBackend, NetBackendError, NetCounters, NetDevice, NetState,
    NotifySource, PacketError, QUEUE_STATE_LEN, Queue, QueueLayout, QueueState, QueueStateError,
    QueueViolation, QueueViolationCounters, QueueViolationKind, REQUEST_HEADER_LEN, RNG_FEATURES,
    RNG_QUEUE_MAX, RNG_STATE_LEN, RNG_STATE_VERSION, RegionLayoutError, Register, RequestError,
    RequestLimits, RestoreError, RngCounters, RngDevice, RngState, SECTOR_SIZE, SLOT_COUNT,
    SOMA_CONTROL_PORT, SOMA_VENDOR_ID, ServiceError, ServiceReport, Slot, SlotRestoreError,
    SlotSnapshot, StatusViolation, StatusWrite, TRANSPORT_STATE_HEADER_LEN, TapBackend,
    TransportConfigError, TransportEvent, TransportState, TransportStateError, TransportViolation,
    TransportViolationCounters, TransportViolationKind, VIRTIO_BLK_DEVICE_ID,
    VIRTIO_BLK_F_BLK_SIZE, VIRTIO_BLK_F_FLUSH, VIRTIO_BLK_F_RO, VIRTIO_BLK_S_IOERR,
    VIRTIO_BLK_S_OK, VIRTIO_BLK_S_UNSUPP, VIRTIO_BLK_T_FLUSH, VIRTIO_BLK_T_GET_ID, VIRTIO_BLK_T_IN,
    VIRTIO_BLK_T_OUT, VIRTIO_F_VERSION_1, VIRTIO_NET_DEVICE_ID, VIRTIO_NET_F_MAC,
    VIRTIO_NET_HDR_LEN, VIRTIO_RNG_DEVICE_ID, VIRTIO_VSOCK_DEVICE_ID, VIRTQ_AVAIL_F_NO_INTERRUPT,
    VIRTQ_DESC_F_INDIRECT, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE, VSOCK_CONFIG_LEN,
    VSOCK_EVENT_QUEUE, VSOCK_EVENT_TRANSPORT_RESET, VSOCK_FEATURES, VSOCK_HDR_LEN,
    VSOCK_OP_CREDIT_REQUEST, VSOCK_OP_CREDIT_UPDATE, VSOCK_OP_INVALID, VSOCK_OP_REQUEST,
    VSOCK_OP_RESPONSE, VSOCK_OP_RST, VSOCK_OP_RW, VSOCK_OP_SHUTDOWN, VSOCK_QUEUE_MAX,
    VSOCK_RX_QUEUE, VSOCK_SHUTDOWN_RCV, VSOCK_SHUTDOWN_SEND, VSOCK_STATE_LEN, VSOCK_STATE_VERSION,
    VSOCK_TX_QUEUE, VSOCK_TYPE_STREAM, VecGuestMemory, VirtioDevice, VsockConfigError,
    VsockCounters, VsockDevice, VsockHeader, VsockState, deliver_events, deliver_rx,
    deliver_vsock_rx, kernel_command_line, parse_request, parse_tx, read_readable, service_queue,
    validate_size, validate_tx, walk_chain, write_writable,
};

#[cfg(unix)]
pub use virtio::{FileBackend, OsEntropy};

/// Whether this build target can run SOMA's initial KVM capability probe.
pub const SUPPORTED_TARGET: bool = cfg!(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
));
