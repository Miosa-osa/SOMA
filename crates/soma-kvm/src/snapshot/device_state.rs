//! Per-device snapshot state for the five fixed virtio-mmio devices.
//!
//! A device state carries transport status, negotiated features, queue geometry and
//! cursors, interrupt status, configuration generation, and the device-specific fields
//! listed by the device-surface contract.
//! It never carries a host descriptor, path, TAP name, socket, key, credit window, packet,
//! or random byte.

mod queue;
mod specific;
#[cfg(test)]
pub(crate) mod tests;

use std::{error::Error, fmt};

pub use queue::{MAX_QUEUES, QueueState};
pub use specific::{BlockState, DeviceSpecific};

use super::{
    WireError,
    wire::{Reader, Writer},
};

pub const DEVICE_COUNT: u8 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    RootBlock,
    OverlayBlock,
    Net,
    Vsock,
    Rng,
}

impl DeviceKind {
    pub const ALL: [Self; 5] = [
        Self::RootBlock,
        Self::OverlayBlock,
        Self::Net,
        Self::Vsock,
        Self::Rng,
    ];

    #[must_use]
    pub const fn slot(self) -> u8 {
        match self {
            Self::RootBlock => 0,
            Self::OverlayBlock => 1,
            Self::Net => 2,
            Self::Vsock => 3,
            Self::Rng => 4,
        }
    }

    #[must_use]
    pub const fn for_slot(slot: u8) -> Option<Self> {
        match slot {
            0 => Some(Self::RootBlock),
            1 => Some(Self::OverlayBlock),
            2 => Some(Self::Net),
            3 => Some(Self::Vsock),
            4 => Some(Self::Rng),
            _ => None,
        }
    }

    /// Fixed queue count from the device-surface contract.
    #[must_use]
    pub const fn queue_count(self) -> usize {
        match self {
            Self::RootBlock | Self::OverlayBlock | Self::Rng => 1,
            Self::Net => 2,
            Self::Vsock => 3,
        }
    }
}

/// Modern virtio-mmio transport registers that survive a snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TransportState {
    pub device_status: u8,
    /// Bit 0: used buffer; bit 1: configuration change. Other bits are invalid.
    pub interrupt_status: u8,
    pub config_generation: u32,
    pub queue_select: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceStateError {
    Wire(WireError),
    UnknownKind(u8),
    KindSlotMismatch { slot: u8, kind: DeviceKind },
    QueueCount { kind: DeviceKind, count: usize },
    InvalidQueue { index: usize, field: &'static str },
    InvalidField { field: &'static str, value: u64 },
    SpecificMismatch(DeviceKind),
}

impl fmt::Display for DeviceStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "device state wire error: {error}"),
            Self::UnknownKind(code) => write!(formatter, "unknown device kind {code}"),
            Self::KindSlotMismatch { slot, kind } => {
                write!(formatter, "device {kind:?} cannot occupy slot {slot}")
            }
            Self::QueueCount { kind, count } => {
                write!(
                    formatter,
                    "{kind:?} requires a fixed queue count, got {count}"
                )
            }
            Self::InvalidQueue { index, field } => {
                write!(formatter, "queue {index} field {field} is invalid")
            }
            Self::InvalidField { field, value } => {
                write!(
                    formatter,
                    "device field {field} has invalid value {value:#x}"
                )
            }
            Self::SpecificMismatch(kind) => {
                write!(formatter, "device-specific state does not match {kind:?}")
            }
        }
    }
}

impl Error for DeviceStateError {}

impl From<WireError> for DeviceStateError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

/// Complete state of one device slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceState {
    kind: DeviceKind,
    transport: TransportState,
    negotiated_features: u64,
    queues: Vec<QueueState>,
    specific: DeviceSpecific,
}

impl DeviceState {
    /// Validates the fixed queue count, queue invariants, interrupt bits, and the
    /// device-specific fields.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DeviceStateError`].
    pub fn new(
        kind: DeviceKind,
        transport: TransportState,
        negotiated_features: u64,
        queues: Vec<QueueState>,
        specific: DeviceSpecific,
    ) -> Result<Self, DeviceStateError> {
        if queues.len() != kind.queue_count() {
            return Err(DeviceStateError::QueueCount {
                kind,
                count: queues.len(),
            });
        }
        for (index, queue) in queues.iter().enumerate() {
            queue.validate(index)?;
        }
        if transport.interrupt_status & !0b11 != 0 {
            return Err(DeviceStateError::InvalidField {
                field: "interrupt_status",
                value: u64::from(transport.interrupt_status),
            });
        }
        specific.validate_for(kind)?;
        Ok(Self {
            kind,
            transport,
            negotiated_features,
            queues,
            specific,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> DeviceKind {
        self.kind
    }

    #[must_use]
    pub const fn transport(&self) -> TransportState {
        self.transport
    }

    #[must_use]
    pub const fn negotiated_features(&self) -> u64 {
        self.negotiated_features
    }

    #[must_use]
    pub fn queues(&self) -> &[QueueState] {
        &self.queues
    }

    #[must_use]
    pub const fn specific(&self) -> DeviceSpecific {
        self.specific
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(128);
        writer.put_u8(self.kind.slot());
        writer.put_u8(self.transport.device_status);
        writer.put_u8(self.transport.interrupt_status);
        writer.put_u32(self.transport.config_generation);
        writer.put_u16(self.transport.queue_select);
        writer.put_u64(self.negotiated_features);
        writer.put_u8(u8::try_from(self.queues.len()).unwrap_or(u8::MAX));
        for queue in &self.queues {
            queue.write(&mut writer);
        }
        self.specific.write(&mut writer);
        writer.finish()
    }

    /// Decodes one device section payload and requires it to belong to `slot`.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DeviceStateError`] for malformed, mismatched, or trailing input.
    pub fn decode_for_slot(slot: u8, bytes: &[u8]) -> Result<Self, DeviceStateError> {
        let mut reader = Reader::new(bytes);
        let kind_code = reader.u8()?;
        let kind =
            DeviceKind::for_slot(kind_code).ok_or(DeviceStateError::UnknownKind(kind_code))?;
        if kind.slot() != slot {
            return Err(DeviceStateError::KindSlotMismatch { slot, kind });
        }
        let transport = TransportState {
            device_status: reader.u8()?,
            interrupt_status: reader.u8()?,
            config_generation: reader.u32()?,
            queue_select: reader.u16()?,
        };
        let negotiated_features = reader.u64()?;
        let count = usize::from(reader.u8()?);
        if count != kind.queue_count() {
            return Err(DeviceStateError::QueueCount { kind, count });
        }
        let mut queues = Vec::with_capacity(count);
        for _ in 0..count {
            queues.push(QueueState::read(&mut reader)?);
        }
        let specific = DeviceSpecific::read(&mut reader, kind)?;
        reader.finish()?;
        Self::new(kind, transport, negotiated_features, queues, specific)
    }
}
