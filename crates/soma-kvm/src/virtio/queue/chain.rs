//! Hostile-input-safe walking of one split-virtqueue descriptor chain.
//!
//! [`walk_chain`] is a pure function over a [`GuestMemory`] view so that a
//! fuzz target can drive it directly with arbitrary descriptor tables.

use std::fmt;

use crate::virtio::guest_memory::{GuestAddress, GuestMemory};

/// Descriptor flag: the `next` field is valid.
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Descriptor flag: the buffer is device-writable.
pub const VIRTQ_DESC_F_WRITE: u16 = 2;
/// Descriptor flag: the buffer holds an indirect table (unsupported in v1).
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;
/// Size of one split-ring descriptor in bytes.
pub const DESCRIPTOR_SIZE: u64 = 16;
/// Largest queue size the split-ring format permits.
pub const MAX_QUEUE_SIZE: u16 = 32768;

const KNOWN_FLAGS: u16 = VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_INDIRECT;

/// One raw split-ring descriptor as stored in guest memory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Descriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

impl Descriptor {
    /// Encodes the descriptor in the guest little-endian layout.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 16] {
        let mut raw = [0u8; 16];
        raw[0..8].copy_from_slice(&self.addr.to_le_bytes());
        raw[8..12].copy_from_slice(&self.len.to_le_bytes());
        raw[12..14].copy_from_slice(&self.flags.to_le_bytes());
        raw[14..16].copy_from_slice(&self.next.to_le_bytes());
        raw
    }

    /// Decodes a descriptor from the guest little-endian layout.
    #[must_use]
    pub const fn from_bytes(raw: [u8; 16]) -> Self {
        let [
            a0,
            a1,
            a2,
            a3,
            a4,
            a5,
            a6,
            a7,
            l0,
            l1,
            l2,
            l3,
            f0,
            f1,
            n0,
            n1,
        ] = raw;
        Self {
            addr: u64::from_le_bytes([a0, a1, a2, a3, a4, a5, a6, a7]),
            len: u32::from_le_bytes([l0, l1, l2, l3]),
            flags: u16::from_le_bytes([f0, f1]),
            next: u16::from_le_bytes([n0, n1]),
        }
    }
}

/// One validated buffer segment of a chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChainSegment {
    pub addr: GuestAddress,
    pub len: u32,
    pub writable: bool,
}

/// A fully validated descriptor chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorChain {
    head: u16,
    segments: Vec<ChainSegment>,
    readable_len: u64,
    writable_len: u64,
}

impl DescriptorChain {
    /// The head index the driver placed in the available ring.
    #[must_use]
    pub const fn head(&self) -> u16 {
        self.head
    }

    /// All segments in chain order; readable segments precede writable ones.
    #[must_use]
    pub fn segments(&self) -> &[ChainSegment] {
        &self.segments
    }

    /// Device-readable segments only.
    pub fn readable(&self) -> impl Iterator<Item = &ChainSegment> {
        self.segments.iter().filter(|segment| !segment.writable)
    }

    /// Device-writable segments only.
    pub fn writable(&self) -> impl Iterator<Item = &ChainSegment> {
        self.segments.iter().filter(|segment| segment.writable)
    }

    /// Total validated device-readable bytes.
    #[must_use]
    pub const fn readable_len(&self) -> u64 {
        self.readable_len
    }

    /// Total validated device-writable bytes; the cap for any used length.
    #[must_use]
    pub const fn writable_len(&self) -> u64 {
        self.writable_len
    }
}

/// Host-configured caps applied before any allocation or I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChainLimits {
    /// Maximum descriptors in one chain; also capped by the queue size.
    pub max_descriptors: u16,
    /// Maximum aggregate bytes across all segments.
    pub max_bytes: u64,
}

/// Why a chain was rejected; carries only indexes and limits, never bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainViolation {
    InvalidQueueSize { size: u16 },
    IndexOutOfRange { index: u16, size: u16 },
    DescriptorUnreadable { index: u16 },
    RepeatedIndex { index: u16 },
    TooLong { limit: u16 },
    Indirect { index: u16 },
    UnknownFlags { index: u16 },
    ZeroLength { index: u16 },
    AddressOverflow { index: u16 },
    OutOfRegion { index: u16 },
    ReadableAfterWritable { index: u16 },
    BytesExceeded { limit: u64 },
}

impl fmt::Display for ChainViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "descriptor chain violation: {self:?}")
    }
}

impl std::error::Error for ChainViolation {}

/// Walks the chain starting at `head` in the descriptor table at `table`.
///
/// The walk is bounded by `queue_size`, `limits`, and a visited bitmap, so a
/// hostile table cannot cause a loop, unbounded allocation, or an out-of-range
/// access.
///
/// # Errors
/// Returns the first violation encountered in chain order.
pub fn walk_chain<M: GuestMemory + ?Sized>(
    mem: &M,
    table: GuestAddress,
    queue_size: u16,
    head: u16,
    limits: ChainLimits,
) -> Result<DescriptorChain, ChainViolation> {
    if queue_size == 0 || queue_size > MAX_QUEUE_SIZE || !queue_size.is_power_of_two() {
        return Err(ChainViolation::InvalidQueueSize { size: queue_size });
    }
    let max_descriptors = limits.max_descriptors.min(queue_size);
    let mut visited = vec![0u64; usize::from(queue_size).div_ceil(64)];
    let mut segments = Vec::new();
    let mut readable_len = 0u64;
    let mut writable_len = 0u64;
    let mut index = head;
    loop {
        if index >= queue_size {
            return Err(ChainViolation::IndexOutOfRange {
                index,
                size: queue_size,
            });
        }
        let (word, bit) = (usize::from(index / 64), index % 64);
        if visited[word] & (1u64 << bit) != 0 {
            return Err(ChainViolation::RepeatedIndex { index });
        }
        visited[word] |= 1u64 << bit;
        if segments.len() >= usize::from(max_descriptors) {
            return Err(ChainViolation::TooLong {
                limit: max_descriptors,
            });
        }
        let descriptor = read_descriptor(mem, table, index)?;
        if descriptor.flags & VIRTQ_DESC_F_INDIRECT != 0 {
            return Err(ChainViolation::Indirect { index });
        }
        if descriptor.flags & !KNOWN_FLAGS != 0 {
            return Err(ChainViolation::UnknownFlags { index });
        }
        if descriptor.len == 0 {
            return Err(ChainViolation::ZeroLength { index });
        }
        let addr = GuestAddress(descriptor.addr);
        let len = u64::from(descriptor.len);
        if addr.checked_add(len).is_none() {
            return Err(ChainViolation::AddressOverflow { index });
        }
        if mem.check_range(addr, len).is_err() {
            return Err(ChainViolation::OutOfRegion { index });
        }
        let writable = descriptor.flags & VIRTQ_DESC_F_WRITE != 0;
        if !writable && writable_len > 0 {
            return Err(ChainViolation::ReadableAfterWritable { index });
        }
        let total = if writable {
            writable_len = writable_len.saturating_add(len);
            readable_len.saturating_add(writable_len)
        } else {
            readable_len = readable_len.saturating_add(len);
            readable_len
        };
        if total > limits.max_bytes {
            return Err(ChainViolation::BytesExceeded {
                limit: limits.max_bytes,
            });
        }
        segments.push(ChainSegment {
            addr,
            len: descriptor.len,
            writable,
        });
        if descriptor.flags & VIRTQ_DESC_F_NEXT == 0 {
            break;
        }
        index = descriptor.next;
    }
    Ok(DescriptorChain {
        head,
        segments,
        readable_len,
        writable_len,
    })
}

fn read_descriptor<M: GuestMemory + ?Sized>(
    mem: &M,
    table: GuestAddress,
    index: u16,
) -> Result<Descriptor, ChainViolation> {
    let unreadable = ChainViolation::DescriptorUnreadable { index };
    let offset = u64::from(index) * DESCRIPTOR_SIZE;
    let addr = table.checked_add(offset).ok_or(unreadable)?;
    let mut raw = [0u8; 16];
    mem.read_bytes(addr, &mut raw).map_err(|_| unreadable)?;
    Ok(Descriptor::from_bytes(raw))
}

#[cfg(test)]
mod tests;
