//! The fixed v1 device table: one 4 KiB page and one GSI per slot, from the
//! minimal device surface. Every address, interrupt, identifier, and the
//! kernel command-line fragment derive from this single table.

use crate::virtio::transport::registers::{MMIO_PAGE_SIZE, REG_QUEUE_NOTIFY};

/// First byte of the MMIO window; above the 3 GiB RAM ceiling.
pub const MMIO_WINDOW_BASE: u64 = 0xd000_0000;
/// Number of device slots.
pub const SLOT_COUNT: usize = 5;
/// GSI of slot 0; slots use consecutive edge-triggered routes.
pub const FIRST_GSI: u32 = 5;

/// One of the five fixed device slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// Immutable EROFS root block device.
    Root = 0,
    /// Private writable overlay block device.
    Overlay = 1,
    /// Network device.
    Net = 2,
    /// Vsock control device.
    Vsock = 3,
    /// Entropy device.
    Rng = 4,
}

impl Slot {
    /// Every slot in table order.
    pub const ALL: [Self; SLOT_COUNT] =
        [Self::Root, Self::Overlay, Self::Net, Self::Vsock, Self::Rng];

    /// Table index.
    #[must_use]
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// First byte of the slot's page.
    #[must_use]
    pub const fn base(self) -> u64 {
        MMIO_WINDOW_BASE + MMIO_PAGE_SIZE * (self as u64)
    }

    /// Last byte of the slot's page, inclusive.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.base() + MMIO_PAGE_SIZE - 1
    }

    /// The guest-physical address of the slot's `QueueNotify` register.
    #[must_use]
    pub const fn notify_addr(self) -> u64 {
        self.base() + REG_QUEUE_NOTIFY
    }

    /// Dedicated interrupt line.
    #[must_use]
    pub const fn gsi(self) -> u32 {
        FIRST_GSI + (self as u32)
    }

    /// Virtio device identifier the slot must report.
    #[must_use]
    pub const fn device_id(self) -> u32 {
        match self {
            Self::Root | Self::Overlay => 2,
            Self::Net => 1,
            Self::Vsock => 19,
            Self::Rng => 4,
        }
    }

    /// Fixed queue count.
    #[must_use]
    pub const fn queue_count(self) -> u16 {
        match self {
            Self::Root | Self::Overlay | Self::Rng => 1,
            Self::Net => 2,
            Self::Vsock => 3,
        }
    }

    /// Resolves a guest-physical address to a slot and page offset.
    #[must_use]
    pub fn from_gpa(gpa: u64) -> Option<(Self, u64)> {
        let relative = gpa.checked_sub(MMIO_WINDOW_BASE)?;
        let index = relative / MMIO_PAGE_SIZE;
        let slot = Self::ALL.get(usize::try_from(index).ok()?)?;
        Some((*slot, relative % MMIO_PAGE_SIZE))
    }

    /// The `virtio_mmio.device=` declaration for this slot.
    #[must_use]
    pub fn command_line_entry(self) -> String {
        format!(
            "virtio_mmio.device=4K@{:#x}:{}:{}",
            self.base(),
            self.gsi(),
            self.index()
        )
    }
}

/// The complete kernel command-line fragment for the five devices.
#[must_use]
pub fn kernel_command_line() -> String {
    Slot::ALL
        .iter()
        .map(|slot| slot.command_line_entry())
        .collect::<Vec<_>>()
        .join(" ")
}
