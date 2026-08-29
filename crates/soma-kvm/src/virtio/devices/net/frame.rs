//! The 12-byte `virtio_net_hdr_v1` and Ethernet frame bounds.
//!
//! No offload or mergeable-buffer feature is offered, so every header field
//! the guest sends must be zero and every frame is one complete Ethernet
//! frame within the configured bounds.

use std::fmt;

use crate::virtio::devices::segments::read_readable;
use crate::virtio::guest_memory::{GuestMemory, GuestMemoryError};
use crate::virtio::queue::chain::DescriptorChain;

/// Length of `virtio_net_hdr_v1` when no mergeable buffers are negotiated.
pub const VIRTIO_NET_HDR_LEN: usize = 12;
/// Smallest frame accepted: one Ethernet header.
pub const MIN_FRAME_LEN: usize = 14;
/// Largest frame accepted: 1500-byte MTU plus the Ethernet header.
pub const MAX_FRAME_LEN: usize = 1514;

/// Why a transmit chain was dropped; carries lengths only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    /// Fewer than header plus minimum frame readable bytes.
    TooShort { len: u64 },
    /// More than header plus maximum frame readable bytes.
    TooLong { len: u64 },
    /// A transmit chain must be device-readable only.
    WritableSegment,
    /// A header field that must be zero is not.
    HeaderNonZero,
    /// The validated header could not be read.
    Memory(GuestMemoryError),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "network frame rejected: {self:?}")
    }
}

impl std::error::Error for FrameError {}

/// Validates a transmit chain and returns the frame length after the header.
///
/// # Errors
/// Returns the typed rejection; the caller drops the chain with a counter.
pub fn validate_tx<M: GuestMemory + ?Sized>(
    mem: &M,
    chain: &DescriptorChain,
) -> Result<usize, FrameError> {
    if chain.writable_len() != 0 {
        return Err(FrameError::WritableSegment);
    }
    let len = chain.readable_len();
    let min = u64::try_from(VIRTIO_NET_HDR_LEN + MIN_FRAME_LEN).unwrap_or(u64::MAX);
    let max = u64::try_from(VIRTIO_NET_HDR_LEN + MAX_FRAME_LEN).unwrap_or(u64::MAX);
    if len < min {
        return Err(FrameError::TooShort { len });
    }
    if len > max {
        return Err(FrameError::TooLong { len });
    }
    let mut header = [0u8; VIRTIO_NET_HDR_LEN];
    read_readable(mem, chain, 0, &mut header).map_err(FrameError::Memory)?;
    if header != [0u8; VIRTIO_NET_HDR_LEN] {
        return Err(FrameError::HeaderNonZero);
    }
    let hdr = u64::try_from(VIRTIO_NET_HDR_LEN).unwrap_or(u64::MAX);
    usize::try_from(len - hdr).map_err(|_| FrameError::TooLong { len })
}

/// Whether a received frame length is deliverable.
#[must_use]
pub const fn rx_frame_ok(len: usize) -> bool {
    len >= MIN_FRAME_LEN && len <= MAX_FRAME_LEN
}
