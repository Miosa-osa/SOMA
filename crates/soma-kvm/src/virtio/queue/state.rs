//! Serializable split-virtqueue state for snapshot capture and restore.
//!
//! The encoding is a fixed 32-byte little-endian record.
//! Decoding validates structure only; semantic validation against device
//! limits and guest memory happens in [`Queue::restore`], which lives here to keep `queue.rs` small.

use std::fmt;

use crate::virtio::guest_memory::{GuestAddress, GuestMemory};
use crate::virtio::queue::{Queue, violation::QueueViolation};

/// Encoded length of one [`QueueState`] record.
pub const QUEUE_STATE_LEN: usize = 32;

/// Snapshot-visible queue state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueState {
    pub size: u16,
    pub ready: bool,
    pub activated: bool,
    pub desc: u64,
    pub avail: u64,
    pub used: u64,
    pub next_avail: u16,
    pub next_used: u16,
}

/// Why an encoded queue state was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueStateError {
    /// The record is not exactly [`QUEUE_STATE_LEN`] bytes.
    Length { actual: usize },
    /// A flag byte holds a value other than zero or one.
    InvalidFlag { offset: usize },
    /// A reserved byte is nonzero.
    ReservedNonZero { offset: usize },
}

impl fmt::Display for QueueStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "queue state rejected: {self:?}")
    }
}

impl std::error::Error for QueueStateError {}

impl QueueState {
    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; QUEUE_STATE_LEN] {
        let mut raw = [0u8; QUEUE_STATE_LEN];
        raw[0..2].copy_from_slice(&self.size.to_le_bytes());
        raw[2] = u8::from(self.ready);
        raw[3] = u8::from(self.activated);
        raw[4..12].copy_from_slice(&self.desc.to_le_bytes());
        raw[12..20].copy_from_slice(&self.avail.to_le_bytes());
        raw[20..28].copy_from_slice(&self.used.to_le_bytes());
        raw[28..30].copy_from_slice(&self.next_avail.to_le_bytes());
        raw[30..32].copy_from_slice(&self.next_used.to_le_bytes());
        raw
    }

    /// Decodes one record with exact-length and flag checks.
    ///
    /// # Errors
    /// Rejects wrong lengths and invalid flag bytes.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, QueueStateError> {
        if raw.len() != QUEUE_STATE_LEN {
            return Err(QueueStateError::Length { actual: raw.len() });
        }
        let flag = |offset: usize| match raw[offset] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(QueueStateError::InvalidFlag { offset }),
        };
        let u16_at = |offset: usize| u16::from_le_bytes([raw[offset], raw[offset + 1]]);
        Ok(Self {
            size: u16_at(0),
            ready: flag(2)?,
            activated: flag(3)?,
            desc: le_u64(&raw[4..12]),
            avail: le_u64(&raw[12..20]),
            used: le_u64(&raw[20..28]),
            next_avail: u16_at(28),
            next_used: u16_at(30),
        })
    }
}

/// Little-endian decode of at most eight bytes without slice-to-array panics.
#[must_use]
pub fn le_u64(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte))
}

impl Queue {
    /// Rebuilds a queue from captured state, revalidating every invariant.
    ///
    /// # Errors
    /// Rejects an invalid maximum, size, geometry, or flag combination.
    pub fn restore<M: GuestMemory + ?Sized>(
        mem: &M,
        max_size: u16,
        state: QueueState,
    ) -> Result<Self, QueueViolation> {
        let mut queue = Self::new(max_size)?;
        queue.set_size(state.size)?;
        queue.desc = GuestAddress(state.desc);
        queue.avail = GuestAddress(state.avail);
        queue.used = GuestAddress(state.used);
        if state.ready && !state.activated {
            return Err(QueueViolation::InconsistentState);
        }
        if !state.activated && (state.next_avail != 0 || state.next_used != 0) {
            return Err(QueueViolation::InconsistentState);
        }
        if state.ready {
            queue.activate(mem)?;
        }
        queue.activated = state.activated;
        queue.next_avail = state.next_avail;
        queue.next_used = state.next_used;
        Ok(queue)
    }
}

#[cfg(test)]
mod tests;
