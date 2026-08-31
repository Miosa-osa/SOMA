//! What one bus access can be, and what it can be rejected for.
//!
//! These are the vocabulary of the bus rather than part of it: a configuration rejection, an
//! access rejection, the side effect of an accepted write, and the two seams through which the
//! machine layer supplies interrupts and notification registration.

use std::fmt;

use crate::virtio::devices::block::BlockRole;
use crate::virtio::guest_memory::GuestAddress;
use crate::virtio::transport::violation::TransportViolation;
use crate::virtio::transport::{TransportConfigError, TransportEvent};

use super::Slot;

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
    /// No slot covers this address, or the slot's device was never built.
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
