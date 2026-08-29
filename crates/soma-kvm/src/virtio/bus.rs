//! The MMIO bus: checked interval dispatch over the five fixed transports.
//!
//! The bus owns the transports and routes reads, writes, notifications,
//! inbound delivery, and snapshot records by slot. It performs no KVM call;
//! ioeventfd and irqfd registration are the [`NotifySource`] and
//! [`IrqSink`] seams the machine layer implements.

pub mod slots;
pub mod table;

use std::fmt;

use crate::virtio::devices::block::{BlockDevice, BlockRole};
use crate::virtio::devices::net::NetDevice;
use crate::virtio::devices::rng::RngDevice;
use crate::virtio::devices::vsock::VsockDevice;
use crate::virtio::guest_memory::{GuestAddress, GuestMemory};
use crate::virtio::transport::registers::AccessWidth;
use crate::virtio::transport::violation::TransportViolation;
use crate::virtio::transport::{MmioTransport, TransportConfigError, TransportEvent};
pub use table::{FIRST_GSI, MMIO_WINDOW_BASE, SLOT_COUNT, Slot, kernel_command_line};

/// Why the bus could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusConfigError {
    /// A block device was handed to the wrong slot.
    BlockRole { slot: Slot, role: BlockRole },
    /// A transport rejected its device.
    Transport {
        slot: Slot,
        error: TransportConfigError,
    },
}

impl fmt::Display for BusConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "bus configuration rejected: {self:?}")
    }
}

impl std::error::Error for BusConfigError {}

/// A rejected bus access; the machine treats an unmapped address as fatal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusViolation {
    /// No slot covers this address.
    UnmappedAddress { gpa: u64 },
    /// The slot's transport rejected the access; it is already counted.
    Transport {
        slot: Slot,
        violation: TransportViolation,
    },
}

impl fmt::Display for BusViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "bus access rejected: {self:?}")
    }
}

impl std::error::Error for BusViolation {}

/// The side effect of one accepted write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusEvent {
    pub slot: Slot,
    pub event: TransportEvent,
}

impl BusEvent {
    /// The queue the driver notified, if this write was a notification.
    #[must_use]
    pub const fn notify(&self) -> Option<(Slot, u16)> {
        match self.event {
            TransportEvent::QueueNotify(queue) => Some((self.slot, queue)),
            _ => None,
        }
    }
}

/// Where the machine layer delivers a device interrupt (irqfd later).
pub trait IrqSink {
    type Error;
    /// Signals one edge on `gsi`.
    ///
    /// # Errors
    /// Returns the machine-specific failure.
    fn signal(&mut self, gsi: u32) -> Result<(), Self::Error>;
}

/// Where the machine layer registers queue-notify addresses (ioeventfd later).
pub trait NotifySource {
    type Error;
    /// Registers a 32-bit write of `queue` at `addr` as a notification for `(slot, queue)`.
    ///
    /// # Errors
    /// Returns the machine-specific failure.
    fn register(&mut self, addr: GuestAddress, slot: Slot, queue: u16) -> Result<(), Self::Error>;
}

/// The five device models in slot order, before they are bound to transports.
pub struct BusDevices {
    pub root: BlockDevice,
    pub overlay: BlockDevice,
    pub net: NetDevice,
    pub vsock: VsockDevice,
    pub rng: RngDevice,
}

/// The five transports at their fixed pages.
pub struct MmioBus {
    root: MmioTransport<BlockDevice>,
    overlay: MmioTransport<BlockDevice>,
    net: MmioTransport<NetDevice>,
    vsock: MmioTransport<VsockDevice>,
    rng: MmioTransport<RngDevice>,
}

macro_rules! with_slot {
    ($bus:expr, $slot:expr, |$t:ident| $body:expr) => {
        match $slot {
            Slot::Root => {
                let $t = &mut $bus.root;
                $body
            }
            Slot::Overlay => {
                let $t = &mut $bus.overlay;
                $body
            }
            Slot::Net => {
                let $t = &mut $bus.net;
                $body
            }
            Slot::Vsock => {
                let $t = &mut $bus.vsock;
                $body
            }
            Slot::Rng => {
                let $t = &mut $bus.rng;
                $body
            }
        }
    };
}
pub(super) use with_slot;

impl MmioBus {
    /// Binds the five device models to fresh transports.
    ///
    /// # Errors
    /// Rejects a block device in the wrong slot or a transport misconfiguration.
    pub fn new(devices: BusDevices) -> Result<Self, BusConfigError> {
        check_role(Slot::Root, &devices.root, BlockRole::ImmutableRoot)?;
        check_role(Slot::Overlay, &devices.overlay, BlockRole::PrivateOverlay)?;
        Ok(Self {
            root: transport(Slot::Root, devices.root)?,
            overlay: transport(Slot::Overlay, devices.overlay)?,
            net: transport(Slot::Net, devices.net)?,
            vsock: transport(Slot::Vsock, devices.vsock)?,
            rng: transport(Slot::Rng, devices.rng)?,
        })
    }

    /// Reads from a transport register or configuration byte range.
    ///
    /// # Errors
    /// Returns the typed rejection; the machine decides the observed value.
    pub fn dispatch_read(&mut self, gpa: u64, width: AccessWidth) -> Result<u64, BusViolation> {
        let (slot, offset) = Slot::from_gpa(gpa).ok_or(BusViolation::UnmappedAddress { gpa })?;
        with_slot!(self, slot, |t| t.read(offset, width))
            .map_err(|violation| BusViolation::Transport { slot, violation })
    }

    /// Applies a driver write.
    ///
    /// # Errors
    /// Returns the typed rejection; an accepted write never partially mutates state.
    pub fn dispatch_write<M: GuestMemory + ?Sized>(
        &mut self,
        gpa: u64,
        width: AccessWidth,
        value: u64,
        mem: &M,
    ) -> Result<BusEvent, BusViolation> {
        let (slot, offset) = Slot::from_gpa(gpa).ok_or(BusViolation::UnmappedAddress { gpa })?;
        let event = with_slot!(self, slot, |t| t.write(offset, width, value, mem))
            .map_err(|violation| BusViolation::Transport { slot, violation })?;
        Ok(BusEvent { slot, event })
    }

    /// Whether the slot has unacknowledged interrupt status.
    #[must_use]
    pub fn interrupt_pending(&self, slot: Slot) -> bool {
        self.interrupt_status(slot) != 0
    }

    /// Raw interrupt status of a slot.
    #[must_use]
    pub fn interrupt_status(&self, slot: Slot) -> u32 {
        match slot {
            Slot::Root => self.root.interrupt_status(),
            Slot::Overlay => self.overlay.interrupt_status(),
            Slot::Net => self.net.interrupt_status(),
            Slot::Vsock => self.vsock.interrupt_status(),
            Slot::Rng => self.rng.interrupt_status(),
        }
    }

    /// Signals the slot's GSI through the sink.
    ///
    /// # Errors
    /// Forwards the sink's failure.
    pub fn signal<S: IrqSink>(&self, slot: Slot, sink: &mut S) -> Result<(), S::Error> {
        sink.signal(slot.gsi())
    }

    /// Registers every `(slot, queue)` notification address with the source.
    ///
    /// # Errors
    /// Forwards the source's failure.
    pub fn register_notify_sources<N: NotifySource>(&self, source: &mut N) -> Result<(), N::Error> {
        for slot in Slot::ALL {
            for queue in 0..slot.queue_count() {
                source.register(GuestAddress(slot.notify_addr()), slot, queue)?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn root(&self) -> &MmioTransport<BlockDevice> {
        &self.root
    }

    #[must_use]
    pub const fn overlay(&self) -> &MmioTransport<BlockDevice> {
        &self.overlay
    }

    #[must_use]
    pub const fn net(&self) -> &MmioTransport<NetDevice> {
        &self.net
    }

    pub const fn net_mut(&mut self) -> &mut MmioTransport<NetDevice> {
        &mut self.net
    }

    #[must_use]
    pub const fn vsock(&self) -> &MmioTransport<VsockDevice> {
        &self.vsock
    }

    pub const fn vsock_mut(&mut self) -> &mut MmioTransport<VsockDevice> {
        &mut self.vsock
    }

    #[must_use]
    pub const fn rng(&self) -> &MmioTransport<RngDevice> {
        &self.rng
    }
}

fn check_role(slot: Slot, device: &BlockDevice, expected: BlockRole) -> Result<(), BusConfigError> {
    if device.role() == expected {
        Ok(())
    } else {
        Err(BusConfigError::BlockRole {
            slot,
            role: device.role(),
        })
    }
}

fn transport<D: crate::virtio::device::VirtioDevice>(
    slot: Slot,
    device: D,
) -> Result<MmioTransport<D>, BusConfigError> {
    MmioTransport::new(device).map_err(|error| BusConfigError::Transport { slot, error })
}

#[cfg(test)]
mod tests;
