//! Modern virtio-mmio (version 2) register offsets and decoding.

/// `MagicValue`: `"virt"` little-endian.
pub const MAGIC_VALUE: u32 = 0x7472_6976;
/// Transport version reported at `Version`.
pub const MMIO_VERSION: u32 = 2;
/// Size of one transport page.
pub const MMIO_PAGE_SIZE: u64 = 0x1000;
/// First byte of the device-specific configuration space.
pub const CONFIG_OFFSET: u64 = 0x100;

pub const REG_MAGIC_VALUE: u64 = 0x000;
pub const REG_VERSION: u64 = 0x004;
pub const REG_DEVICE_ID: u64 = 0x008;
pub const REG_VENDOR_ID: u64 = 0x00c;
pub const REG_DEVICE_FEATURES: u64 = 0x010;
pub const REG_DEVICE_FEATURES_SEL: u64 = 0x014;
pub const REG_DRIVER_FEATURES: u64 = 0x020;
pub const REG_DRIVER_FEATURES_SEL: u64 = 0x024;
pub const REG_QUEUE_SEL: u64 = 0x030;
pub const REG_QUEUE_NUM_MAX: u64 = 0x034;
pub const REG_QUEUE_NUM: u64 = 0x038;
pub const REG_QUEUE_READY: u64 = 0x044;
pub const REG_QUEUE_NOTIFY: u64 = 0x050;
pub const REG_INTERRUPT_STATUS: u64 = 0x060;
pub const REG_INTERRUPT_ACK: u64 = 0x064;
pub const REG_STATUS: u64 = 0x070;
pub const REG_QUEUE_DESC_LOW: u64 = 0x080;
pub const REG_QUEUE_DESC_HIGH: u64 = 0x084;
pub const REG_QUEUE_DRIVER_LOW: u64 = 0x090;
pub const REG_QUEUE_DRIVER_HIGH: u64 = 0x094;
pub const REG_QUEUE_DEVICE_LOW: u64 = 0x0a0;
pub const REG_QUEUE_DEVICE_HIGH: u64 = 0x0a4;
pub const REG_SHM_SEL: u64 = 0x0ac;
pub const REG_SHM_LEN_LOW: u64 = 0x0b0;
pub const REG_SHM_LEN_HIGH: u64 = 0x0b4;
pub const REG_SHM_BASE_LOW: u64 = 0x0b8;
pub const REG_SHM_BASE_HIGH: u64 = 0x0bc;
pub const REG_QUEUE_RESET: u64 = 0x0c0;
pub const REG_CONFIG_GENERATION: u64 = 0x0fc;

/// Width of one MMIO access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessWidth {
    U8,
    U16,
    U32,
    U64,
}

impl AccessWidth {
    /// Number of bytes moved by the access.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
            Self::U64 => 8,
        }
    }
}

/// A decoded transport register.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Register {
    MagicValue,
    Version,
    DeviceId,
    VendorId,
    DeviceFeatures,
    DeviceFeaturesSel,
    DriverFeatures,
    DriverFeaturesSel,
    QueueSel,
    QueueNumMax,
    QueueNum,
    QueueReady,
    QueueNotify,
    InterruptStatus,
    InterruptAck,
    Status,
    QueueDescLow,
    QueueDescHigh,
    QueueDriverLow,
    QueueDriverHigh,
    QueueDeviceLow,
    QueueDeviceHigh,
    ShmSel,
    ShmLenLow,
    ShmLenHigh,
    ShmBaseLow,
    ShmBaseHigh,
    QueueReset,
    ConfigGeneration,
    /// Device-specific configuration space at the given byte offset.
    Config(u64),
}

impl Register {
    /// Decodes a page-relative offset; `None` for reserved or out-of-page offsets.
    #[must_use]
    pub const fn decode(offset: u64) -> Option<Self> {
        Some(match offset {
            REG_MAGIC_VALUE => Self::MagicValue,
            REG_VERSION => Self::Version,
            REG_DEVICE_ID => Self::DeviceId,
            REG_VENDOR_ID => Self::VendorId,
            REG_DEVICE_FEATURES => Self::DeviceFeatures,
            REG_DEVICE_FEATURES_SEL => Self::DeviceFeaturesSel,
            REG_DRIVER_FEATURES => Self::DriverFeatures,
            REG_DRIVER_FEATURES_SEL => Self::DriverFeaturesSel,
            REG_QUEUE_SEL => Self::QueueSel,
            REG_QUEUE_NUM_MAX => Self::QueueNumMax,
            REG_QUEUE_NUM => Self::QueueNum,
            REG_QUEUE_READY => Self::QueueReady,
            REG_QUEUE_NOTIFY => Self::QueueNotify,
            REG_INTERRUPT_STATUS => Self::InterruptStatus,
            REG_INTERRUPT_ACK => Self::InterruptAck,
            REG_STATUS => Self::Status,
            REG_QUEUE_DESC_LOW => Self::QueueDescLow,
            REG_QUEUE_DESC_HIGH => Self::QueueDescHigh,
            REG_QUEUE_DRIVER_LOW => Self::QueueDriverLow,
            REG_QUEUE_DRIVER_HIGH => Self::QueueDriverHigh,
            REG_QUEUE_DEVICE_LOW => Self::QueueDeviceLow,
            REG_QUEUE_DEVICE_HIGH => Self::QueueDeviceHigh,
            REG_SHM_SEL => Self::ShmSel,
            REG_SHM_LEN_LOW => Self::ShmLenLow,
            REG_SHM_LEN_HIGH => Self::ShmLenHigh,
            REG_SHM_BASE_LOW => Self::ShmBaseLow,
            REG_SHM_BASE_HIGH => Self::ShmBaseHigh,
            REG_QUEUE_RESET => Self::QueueReset,
            REG_CONFIG_GENERATION => Self::ConfigGeneration,
            CONFIG_OFFSET..MMIO_PAGE_SIZE => Self::Config(offset - CONFIG_OFFSET),
            _ => return None,
        })
    }
}
