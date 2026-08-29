//! Split-virtqueue geometry validation: size, alignment, containment, overlap.

use std::fmt;

use crate::virtio::guest_memory::{GuestAddress, GuestMemory};
use crate::virtio::queue::chain::{DESCRIPTOR_SIZE, MAX_QUEUE_SIZE};

/// Required alignment of the descriptor table.
pub const DESC_ALIGN: u64 = 16;
/// Required alignment of the available ring.
pub const AVAIL_ALIGN: u64 = 2;
/// Required alignment of the used ring.
pub const USED_ALIGN: u64 = 4;

/// Why a queue geometry was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutViolation {
    SizeZero,
    SizeNotPowerOfTwo { size: u16 },
    SizeExceedsMax { size: u16, max: u16 },
    DescMisaligned,
    AvailMisaligned,
    UsedMisaligned,
    DescOutOfRegion,
    AvailOutOfRegion,
    UsedOutOfRegion,
    RingsOverlap,
}

impl fmt::Display for LayoutViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "queue layout violation: {self:?}")
    }
}

impl std::error::Error for LayoutViolation {}

/// Validates a driver-selected queue size against the device maximum.
///
/// # Errors
/// Rejects zero, non-power-of-two, and oversized values.
pub fn validate_size(size: u16, max: u16) -> Result<u16, LayoutViolation> {
    if size == 0 {
        return Err(LayoutViolation::SizeZero);
    }
    if !size.is_power_of_two() {
        return Err(LayoutViolation::SizeNotPowerOfTwo { size });
    }
    if size > max || size > MAX_QUEUE_SIZE {
        return Err(LayoutViolation::SizeExceedsMax { size, max });
    }
    Ok(size)
}

/// A validated queue geometry whose rings lie wholly in registered memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueLayout {
    size: u16,
    desc: GuestAddress,
    avail: GuestAddress,
    used: GuestAddress,
}

impl QueueLayout {
    /// Validates size, alignment, containment, and pairwise ring disjointness.
    ///
    /// # Errors
    /// Returns the first violated invariant.
    pub fn validate<M: GuestMemory + ?Sized>(
        mem: &M,
        size: u16,
        max: u16,
        desc: GuestAddress,
        avail: GuestAddress,
        used: GuestAddress,
    ) -> Result<Self, LayoutViolation> {
        let size = validate_size(size, max)?;
        if !desc.is_aligned(DESC_ALIGN) {
            return Err(LayoutViolation::DescMisaligned);
        }
        if !avail.is_aligned(AVAIL_ALIGN) {
            return Err(LayoutViolation::AvailMisaligned);
        }
        if !used.is_aligned(USED_ALIGN) {
            return Err(LayoutViolation::UsedMisaligned);
        }
        let layout = Self {
            size,
            desc,
            avail,
            used,
        };
        let ranges = [
            (desc, layout.desc_len(), LayoutViolation::DescOutOfRegion),
            (avail, layout.avail_len(), LayoutViolation::AvailOutOfRegion),
            (used, layout.used_len(), LayoutViolation::UsedOutOfRegion),
        ];
        for (start, len, violation) in ranges {
            mem.check_range(start, len).map_err(|_| violation)?;
        }
        for (first, second) in [(0, 1), (0, 2), (1, 2)] {
            let (a, a_len, _) = ranges[first];
            let (b, b_len, _) = ranges[second];
            // Containment succeeded, so start + len cannot overflow here.
            if a.0 < b.0 + b_len && b.0 < a.0 + a_len {
                return Err(LayoutViolation::RingsOverlap);
            }
        }
        Ok(layout)
    }

    /// Negotiated queue size.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Descriptor table base.
    #[must_use]
    pub const fn desc(&self) -> GuestAddress {
        self.desc
    }

    /// Available ring base.
    #[must_use]
    pub const fn avail(&self) -> GuestAddress {
        self.avail
    }

    /// Used ring base.
    #[must_use]
    pub const fn used(&self) -> GuestAddress {
        self.used
    }

    /// Descriptor table length: `16 * size`.
    #[must_use]
    pub const fn desc_len(&self) -> u64 {
        self.size as u64 * DESCRIPTOR_SIZE
    }

    /// Available ring length without the event-idx word: `6 + 2 * size`.
    #[must_use]
    pub const fn avail_len(&self) -> u64 {
        6 + 2 * self.size as u64
    }

    /// Used ring length without the event-idx word: `6 + 8 * size`.
    #[must_use]
    pub const fn used_len(&self) -> u64 {
        6 + 8 * self.size as u64
    }
}
