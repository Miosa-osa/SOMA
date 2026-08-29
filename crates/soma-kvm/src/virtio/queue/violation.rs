//! Typed queue violations and bounded saturating counters.

use std::fmt;

use crate::virtio::guest_memory::GuestMemoryError;
use crate::virtio::queue::chain::ChainViolation;
use crate::virtio::queue::layout::LayoutViolation;
use crate::virtio::queue::state::QueueStateError;

/// A rejected queue operation; carries only indexes and limits, never bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueViolation {
    /// Queue work was requested before the queue was ready.
    NotReady,
    /// The driver tried to activate a queue twice without a reset.
    AlreadyActivated,
    /// Size, alignment, or containment failed.
    Layout(LayoutViolation),
    /// A ring access outside registered memory.
    Memory(GuestMemoryError),
    /// The driver advanced the available index by more than the queue size.
    AvailIndexOverrun { pending: u16, size: u16 },
    /// The chain at `head` failed validation.
    Chain {
        head: u16,
        violation: ChainViolation,
    },
    /// The device tried to report more bytes than the chain can hold.
    UsedLengthExceedsCapacity { len: u32, capacity: u64 },
    /// A restored state record was structurally or semantically invalid.
    InvalidState(QueueStateError),
    /// A restored cursor or flag combination is impossible.
    InconsistentState,
}

impl fmt::Display for QueueViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "queue violation: {self:?}")
    }
}

impl std::error::Error for QueueViolation {}

impl From<LayoutViolation> for QueueViolation {
    fn from(violation: LayoutViolation) -> Self {
        Self::Layout(violation)
    }
}

impl From<GuestMemoryError> for QueueViolation {
    fn from(error: GuestMemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Classification used for counting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum QueueViolationKind {
    NotReady = 0,
    AlreadyActivated = 1,
    Layout = 2,
    Memory = 3,
    AvailIndexOverrun = 4,
    Chain = 5,
    UsedLengthExceedsCapacity = 6,
    InvalidState = 7,
    InconsistentState = 8,
}

const KIND_COUNT: usize = 9;

impl QueueViolation {
    /// The counting class of this violation.
    #[must_use]
    pub const fn kind(&self) -> QueueViolationKind {
        match self {
            Self::NotReady => QueueViolationKind::NotReady,
            Self::AlreadyActivated => QueueViolationKind::AlreadyActivated,
            Self::Layout(_) => QueueViolationKind::Layout,
            Self::Memory(_) => QueueViolationKind::Memory,
            Self::AvailIndexOverrun { .. } => QueueViolationKind::AvailIndexOverrun,
            Self::Chain { .. } => QueueViolationKind::Chain,
            Self::UsedLengthExceedsCapacity { .. } => QueueViolationKind::UsedLengthExceedsCapacity,
            Self::InvalidState(_) => QueueViolationKind::InvalidState,
            Self::InconsistentState => QueueViolationKind::InconsistentState,
        }
    }
}

/// Saturating per-class counters; never records guest data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueViolationCounters {
    counts: [u32; KIND_COUNT],
}

impl QueueViolationCounters {
    /// Records one violation.
    pub fn record(&mut self, violation: &QueueViolation) {
        let slot = &mut self.counts[violation.kind() as usize];
        *slot = slot.saturating_add(1);
    }

    /// Count for one class.
    #[must_use]
    pub const fn count(&self, kind: QueueViolationKind) -> u32 {
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
