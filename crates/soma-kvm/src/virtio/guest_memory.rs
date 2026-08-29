//! Bounded guest-physical memory access for virtio transport and queue code.
//!
//! Every access is range-checked against registered regions before any byte
//! moves, so guest-controlled addresses and lengths never reach a host slice
//! unchecked.

use std::{cell::RefCell, fmt};

/// A guest-physical address.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GuestAddress(pub u64);

impl GuestAddress {
    /// The raw guest-physical value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Adds an offset with overflow detection.
    #[must_use]
    pub const fn checked_add(self, offset: u64) -> Option<Self> {
        match self.0.checked_add(offset) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Whether the address is a multiple of `align`.
    #[must_use]
    pub const fn is_aligned(self, align: u64) -> bool {
        align != 0 && self.0.is_multiple_of(align)
    }
}

/// A rejected guest-memory access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestMemoryError {
    /// `addr + len` overflows the guest-physical address space.
    Overflow { addr: GuestAddress, len: u64 },
    /// The range does not lie wholly inside one registered region.
    OutOfRegion { addr: GuestAddress, len: u64 },
}

impl fmt::Display for GuestMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow { addr, len } => {
                write!(formatter, "guest range {:#x}+{len:#x} overflows", addr.0)
            }
            Self::OutOfRegion { addr, len } => {
                write!(
                    formatter,
                    "guest range {:#x}+{len:#x} is unregistered",
                    addr.0
                )
            }
        }
    }
}

impl std::error::Error for GuestMemoryError {}

/// A fixed-width little-endian value that crosses the guest boundary.
pub trait GuestValue: Copy {
    /// Encoded size in bytes; at most eight.
    const SIZE: usize;
    /// Decodes from exactly `SIZE` little-endian bytes.
    fn from_le_slice(bytes: &[u8]) -> Self;
    /// Encodes into exactly `SIZE` little-endian bytes.
    fn write_le(self, out: &mut [u8]);
}

macro_rules! guest_value {
    ($($ty:ty),*) => {$(
        impl GuestValue for $ty {
            const SIZE: usize = size_of::<$ty>();

            fn from_le_slice(bytes: &[u8]) -> Self {
                let mut raw = [0u8; Self::SIZE];
                raw.copy_from_slice(bytes);
                Self::from_le_bytes(raw)
            }

            fn write_le(self, out: &mut [u8]) {
                out.copy_from_slice(&self.to_le_bytes());
            }
        }
    )*};
}

guest_value!(u8, u16, u32, u64);

/// Checked access to registered guest-physical memory.
///
/// Implementations must reject any range that is not wholly inside one
/// registered region.
/// Writes take `&self` because guest memory is shared with the guest.
pub trait GuestMemory {
    /// Validates that `len` bytes at `addr` are readable and writable.
    ///
    /// # Errors
    /// Returns the typed rejection when the range overflows or is unregistered.
    fn check_range(&self, addr: GuestAddress, len: u64) -> Result<(), GuestMemoryError>;

    /// Copies guest bytes into `buf`.
    ///
    /// # Errors
    /// Returns the typed rejection when the range is invalid.
    fn read_bytes(&self, addr: GuestAddress, buf: &mut [u8]) -> Result<(), GuestMemoryError>;

    /// Copies `bytes` into guest memory.
    ///
    /// # Errors
    /// Returns the typed rejection when the range is invalid.
    fn write_bytes(&self, addr: GuestAddress, bytes: &[u8]) -> Result<(), GuestMemoryError>;

    /// Reads one little-endian value.
    ///
    /// # Errors
    /// Returns the typed rejection when the range is invalid.
    fn read_obj_at<T: GuestValue>(&self, addr: GuestAddress) -> Result<T, GuestMemoryError> {
        let mut raw = [0u8; 8];
        let buf = &mut raw[..T::SIZE];
        self.read_bytes(addr, buf)?;
        Ok(T::from_le_slice(buf))
    }

    /// Writes one little-endian value.
    ///
    /// # Errors
    /// Returns the typed rejection when the range is invalid.
    fn write_obj_at<T: GuestValue>(
        &self,
        addr: GuestAddress,
        value: T,
    ) -> Result<(), GuestMemoryError> {
        let mut raw = [0u8; 8];
        let buf = &mut raw[..T::SIZE];
        value.write_le(buf);
        self.write_bytes(addr, buf)
    }
}

/// A rejected in-memory region layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionLayoutError {
    /// A region has zero length.
    EmptyRegion,
    /// A region end overflows the address space.
    Overflow,
    /// Two regions overlap or are not ascending.
    Overlap,
}

impl fmt::Display for RegionLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRegion => "guest memory region is empty",
            Self::Overflow => "guest memory region overflows",
            Self::Overlap => "guest memory regions overlap",
        })
    }
}

impl std::error::Error for RegionLayoutError {}

struct Region {
    start: GuestAddress,
    bytes: RefCell<Vec<u8>>,
}

/// Host-heap guest memory made of ascending non-overlapping regions.
///
/// It exists for tests and fuzz targets; production memory arrives through a
/// separate mapped implementation of [`GuestMemory`].
pub struct VecGuestMemory {
    regions: Vec<Region>,
}

impl VecGuestMemory {
    /// Builds memory from `(start, len)` regions in ascending order.
    ///
    /// # Errors
    /// Rejects empty, overflowing, or overlapping regions.
    pub fn new(layout: &[(GuestAddress, usize)]) -> Result<Self, RegionLayoutError> {
        let mut regions = Vec::with_capacity(layout.len());
        let mut next_free = 0u64;
        for &(start, len) in layout {
            if len == 0 {
                return Err(RegionLayoutError::EmptyRegion);
            }
            let end = start
                .checked_add(u64::try_from(len).map_err(|_| RegionLayoutError::Overflow)?)
                .ok_or(RegionLayoutError::Overflow)?;
            if start.0 < next_free {
                return Err(RegionLayoutError::Overlap);
            }
            next_free = end.0;
            regions.push(Region {
                start,
                bytes: RefCell::new(vec![0; len]),
            });
        }
        Ok(Self { regions })
    }

    /// One region starting at guest address zero.
    ///
    /// # Errors
    /// Rejects a zero length.
    pub fn flat(len: usize) -> Result<Self, RegionLayoutError> {
        Self::new(&[(GuestAddress(0), len)])
    }

    fn locate(&self, addr: GuestAddress, len: u64) -> Result<(&Region, usize), GuestMemoryError> {
        let end = addr
            .checked_add(len)
            .ok_or(GuestMemoryError::Overflow { addr, len })?;
        for region in &self.regions {
            let region_len = region.bytes.borrow().len() as u64;
            let region_end = region.start.0 + region_len;
            if addr.0 >= region.start.0 && end.0 <= region_end {
                let offset =
                    usize::try_from(addr.0 - region.start.0).expect("offset fits region length");
                return Ok((region, offset));
            }
        }
        Err(GuestMemoryError::OutOfRegion { addr, len })
    }
}

fn slice_len(buf_len: usize) -> u64 {
    u64::try_from(buf_len).expect("host slice length fits in u64")
}

impl GuestMemory for VecGuestMemory {
    fn check_range(&self, addr: GuestAddress, len: u64) -> Result<(), GuestMemoryError> {
        self.locate(addr, len).map(|_| ())
    }

    fn read_bytes(&self, addr: GuestAddress, buf: &mut [u8]) -> Result<(), GuestMemoryError> {
        let (region, offset) = self.locate(addr, slice_len(buf.len()))?;
        buf.copy_from_slice(&region.bytes.borrow()[offset..offset + buf.len()]);
        Ok(())
    }

    fn write_bytes(&self, addr: GuestAddress, bytes: &[u8]) -> Result<(), GuestMemoryError> {
        let (region, offset) = self.locate(addr, slice_len(bytes.len()))?;
        region.bytes.borrow_mut()[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
