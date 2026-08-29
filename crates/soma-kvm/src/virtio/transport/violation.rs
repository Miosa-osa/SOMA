//! Typed transport violations and bounded saturating counters.

use std::fmt;

use crate::virtio::device::{ActivateError, ConfigAccessError};
use crate::virtio::queue::violation::QueueViolation;
use crate::virtio::transport::status::StatusViolation;

/// A rejected transport access; carries offsets and classes, never guest bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportViolation {
    /// The offset is reserved or outside the page.
    UnknownRegister { offset: u64 },
    /// A transport register was accessed with a width other than 32 bits.
    WidthMismatch { offset: u64 },
    /// A write-only register was read.
    ReadOfWriteOnly { offset: u64 },
    /// A read-only register was written.
    WriteOfReadOnly { offset: u64 },
    /// Feature or queue configuration was written after the lifecycle locked it.
    ConfigurationLocked { offset: u64 },
    /// The driver accepted bits outside the allowlist or omitted `VERSION_1`.
    FeaturesRejected {
        unsupported: u64,
        missing_version_1: bool,
    },
    /// A status write broke the lifecycle order.
    Status(StatusViolation),
    /// `QueueSel` chose an index the device does not have.
    QueueSelOutOfRange { sel: u32 },
    /// A queue operation was rejected.
    Queue(QueueViolation),
    /// `QueueNotify` arrived before `DRIVER_OK` or after a failure.
    NotifyBeforeDriverOk,
    /// `QueueNotify` named an index outside the device's queue count.
    NotifyOutOfRange { index: u64 },
    /// `QueueNotify` named a queue that is not ready.
    NotifyQueueNotReady { index: u16 },
    /// A configuration-space access fell outside the device's config length.
    ConfigOutOfBounds { offset: u64 },
    /// The device rejected a configuration-space access.
    ConfigAccess(ConfigAccessError),
    /// `InterruptACK` carried bits the transport never raises.
    InterruptAckUnknownBits { value: u64 },
    /// `DRIVER_OK` was set but the device could not activate.
    Activate(ActivateError),
    /// `QueueReset` was written although `VIRTIO_F_RING_RESET` is not offered.
    RingResetUnsupported,
}

impl fmt::Display for TransportViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "transport violation: {self:?}")
    }
}

impl std::error::Error for TransportViolation {}

impl From<QueueViolation> for TransportViolation {
    fn from(violation: QueueViolation) -> Self {
        Self::Queue(violation)
    }
}

/// Classification used for counting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum TransportViolationKind {
    UnknownRegister = 0,
    WidthMismatch = 1,
    ReadOfWriteOnly = 2,
    WriteOfReadOnly = 3,
    ConfigurationLocked = 4,
    FeaturesRejected = 5,
    Status = 6,
    QueueSelOutOfRange = 7,
    Queue = 8,
    NotifyBeforeDriverOk = 9,
    NotifyOutOfRange = 10,
    NotifyQueueNotReady = 11,
    ConfigOutOfBounds = 12,
    ConfigAccess = 13,
    InterruptAckUnknownBits = 14,
    Activate = 15,
    RingResetUnsupported = 16,
}

const KIND_COUNT: usize = 17;

impl TransportViolation {
    /// The counting class of this violation.
    #[must_use]
    pub const fn kind(&self) -> TransportViolationKind {
        match self {
            Self::UnknownRegister { .. } => TransportViolationKind::UnknownRegister,
            Self::WidthMismatch { .. } => TransportViolationKind::WidthMismatch,
            Self::ReadOfWriteOnly { .. } => TransportViolationKind::ReadOfWriteOnly,
            Self::WriteOfReadOnly { .. } => TransportViolationKind::WriteOfReadOnly,
            Self::ConfigurationLocked { .. } => TransportViolationKind::ConfigurationLocked,
            Self::FeaturesRejected { .. } => TransportViolationKind::FeaturesRejected,
            Self::Status(_) => TransportViolationKind::Status,
            Self::QueueSelOutOfRange { .. } => TransportViolationKind::QueueSelOutOfRange,
            Self::Queue(_) => TransportViolationKind::Queue,
            Self::NotifyBeforeDriverOk => TransportViolationKind::NotifyBeforeDriverOk,
            Self::NotifyOutOfRange { .. } => TransportViolationKind::NotifyOutOfRange,
            Self::NotifyQueueNotReady { .. } => TransportViolationKind::NotifyQueueNotReady,
            Self::ConfigOutOfBounds { .. } => TransportViolationKind::ConfigOutOfBounds,
            Self::ConfigAccess(_) => TransportViolationKind::ConfigAccess,
            Self::InterruptAckUnknownBits { .. } => TransportViolationKind::InterruptAckUnknownBits,
            Self::Activate(_) => TransportViolationKind::Activate,
            Self::RingResetUnsupported => TransportViolationKind::RingResetUnsupported,
        }
    }
}

/// Saturating per-class counters; never records guest data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TransportViolationCounters {
    counts: [u32; KIND_COUNT],
}

impl TransportViolationCounters {
    /// Records one violation.
    pub fn record(&mut self, violation: &TransportViolation) {
        let slot = &mut self.counts[violation.kind() as usize];
        *slot = slot.saturating_add(1);
    }

    /// Count for one class.
    #[must_use]
    pub const fn count(&self, kind: TransportViolationKind) -> u32 {
        self.counts[kind as usize]
    }

    /// Sum across classes, saturating.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.counts
            .iter()
            .fold(0u32, |acc, count| acc.saturating_add(*count))
    }
}
