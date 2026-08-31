//! The v1 device table: one 4 KiB page and one GSI per slot, from the minimal device surface.
//! Every address, interrupt, identifier, and the kernel command-line fragment derive from this
//! single table.
//!
//! The five slots are the maximum a machine may expose, not the minimum it must. Two of them
//! carry a capability rather than the ability to run at all: a sandbox that never writes needs
//! no private overlay, and a sandbox that may not reach the network needs no network device.
//! A [`DeviceSet`] says which of those two a Generation declared, and everything downstream,
//! meaning which device models are built, which pages the guest is told about, and which
//! contract digest the snapshot binds, is derived from it.

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

/// Which optional slots one machine declares, and therefore which devices it has at all.
///
/// The root block device, the vsock control device, and the entropy device are not represented
/// here because they are not optional: without a root there is no code to run, without vsock
/// there is no channel in or out, and without entropy a restored guest wakes with the
/// snapshot's stale generator state shared by every other Instance of that snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct DeviceSet {
    overlay: bool,
    net: bool,
}

impl DeviceSet {
    /// Every optional slot present: the maximum surface the contract allows.
    pub const FULL: Self = Self {
        overlay: true,
        net: true,
    };

    /// The set a Generation gets from what it declared.
    #[must_use]
    pub const fn new(overlay: bool, net: bool) -> Self {
        Self { overlay, net }
    }

    /// Whether this machine has a private writable overlay.
    #[must_use]
    pub const fn overlay(self) -> bool {
        self.overlay
    }

    /// Whether this machine has a network device.
    #[must_use]
    pub const fn net(self) -> bool {
        self.net
    }

    /// Whether the slot is present in this machine.
    #[must_use]
    pub const fn has(self, slot: Slot) -> bool {
        match slot {
            Slot::Root | Slot::Vsock | Slot::Rng => true,
            Slot::Overlay => self.overlay,
            Slot::Net => self.net,
        }
    }

    /// Every present slot in table order.
    pub fn present(self) -> impl Iterator<Item = Slot> {
        Slot::ALL.into_iter().filter(move |slot| self.has(*slot))
    }
}

/// The kernel command-line fragment naming exactly the present devices.
///
/// An absent slot keeps its page and its interrupt reserved but is never declared, so the guest
/// never probes it and the addresses of the slots that remain do not move. Two machines that
/// declare different sets therefore differ only by which declarations are missing, which is
/// what makes the difference visible in the command line the manifest binds.
#[must_use]
pub fn kernel_command_line(devices: DeviceSet) -> String {
    devices
        .present()
        .map(Slot::command_line_entry)
        .collect::<Vec<_>>()
        .join(" ")
}
