//! Parsing and validation of one virtio-blk request chain.
//!
//! The parser accepts a validated descriptor chain plus the device limits
//! and returns a typed operation or a typed rejection; it never touches a
//! backend.

use std::fmt;

use crate::virtio::devices::segments::read_readable;
use crate::virtio::guest_memory::{GuestMemory, GuestMemoryError};
use crate::virtio::queue::chain::DescriptorChain;

/// Request type: read from device.
pub const VIRTIO_BLK_T_IN: u32 = 0;
/// Request type: write to device.
pub const VIRTIO_BLK_T_OUT: u32 = 1;
/// Request type: flush the write cache.
pub const VIRTIO_BLK_T_FLUSH: u32 = 4;
/// Request type: read the 20-byte device identity.
pub const VIRTIO_BLK_T_GET_ID: u32 = 8;
/// Status: success.
pub const VIRTIO_BLK_S_OK: u8 = 0;
/// Status: I/O error.
pub const VIRTIO_BLK_S_IOERR: u8 = 1;
/// Status: unsupported request.
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;
/// Bytes per virtio-blk sector; capacity is always in these units.
pub const SECTOR_SIZE: u64 = 512;
/// Fixed request header length.
pub const REQUEST_HEADER_LEN: u64 = 16;
/// Device identity length for `GET_ID`.
pub const BLK_ID_LEN: usize = 20;
/// Largest data region one request may carry.
pub const MAX_REQUEST_BYTES: u64 = 1 << 20;

/// A validated operation on the backing store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockOp {
    /// Read `len` bytes at byte `offset` into the writable area.
    Read { offset: u64, len: u32 },
    /// Write `len` bytes from the readable area (after the header) at `offset`.
    Write { offset: u64, len: u32 },
    /// Flush the write cache.
    Flush,
    /// Fill up to `len` bytes of identity.
    GetId { len: u32 },
}

/// A parsed request; `status_skip` is the writable offset of the status byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRequest {
    pub op: BlockOp,
    pub status_skip: u64,
}

/// Why a request was rejected; carries no guest bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    /// Fewer than 16 readable header bytes.
    HeaderShort,
    /// No writable status byte at the end of the chain.
    NoStatus,
    /// The header could not be read from validated memory.
    Memory(GuestMemoryError),
    /// The type is not allowlisted for this device.
    UnsupportedType { ty: u32 },
    /// A write reached a device that offered `VIRTIO_BLK_F_RO`.
    ReadOnly,
    /// The data region direction or presence does not match the type.
    DirectionMismatch,
    /// `sector * 512` overflows.
    SectorOverflow,
    /// `offset + len` overflows.
    RangeOverflow,
    /// The data length is not a multiple of the sector size or is zero.
    Unaligned { len: u64 },
    /// The request ends past the certified capacity.
    BeyondCapacity,
    /// The data region exceeds [`MAX_REQUEST_BYTES`].
    TooLarge { len: u64 },
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "block request rejected: {self:?}")
    }
}

impl std::error::Error for RequestError {}

impl RequestError {
    /// The status byte the guest observes for this rejection.
    #[must_use]
    pub const fn status(self) -> u8 {
        match self {
            Self::UnsupportedType { .. } => VIRTIO_BLK_S_UNSUPP,
            _ => VIRTIO_BLK_S_IOERR,
        }
    }
}

/// Device limits the parser enforces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestLimits {
    pub capacity_bytes: u64,
    pub read_only: bool,
    pub flush: bool,
}

/// Parses the header and validates the chain shape against `limits`.
///
/// # Errors
/// Returns the typed rejection; the caller writes `error.status()` when the
/// chain has a status byte.
pub fn parse_request<M: GuestMemory + ?Sized>(
    mem: &M,
    chain: &DescriptorChain,
    limits: RequestLimits,
) -> Result<BlockRequest, RequestError> {
    let readable = chain.readable_len();
    let writable = chain.writable_len();
    if writable < 1 {
        return Err(RequestError::NoStatus);
    }
    if readable < REQUEST_HEADER_LEN {
        return Err(RequestError::HeaderShort);
    }
    let mut header = [0u8; 16];
    read_readable(mem, chain, 0, &mut header).map_err(RequestError::Memory)?;
    let ty = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let sector = u64::from_le_bytes(header[8..16].try_into().unwrap_or([0; 8]));
    let extra_readable = readable - REQUEST_HEADER_LEN;
    let data_writable = writable - 1;
    let status_skip = data_writable;
    let op = match ty {
        VIRTIO_BLK_T_IN => {
            if extra_readable != 0 {
                return Err(RequestError::DirectionMismatch);
            }
            let (offset, len) = data_range(sector, data_writable, limits.capacity_bytes)?;
            BlockOp::Read { offset, len }
        }
        VIRTIO_BLK_T_OUT if limits.read_only => return Err(RequestError::ReadOnly),
        VIRTIO_BLK_T_OUT => {
            if data_writable != 0 {
                return Err(RequestError::DirectionMismatch);
            }
            let (offset, len) = data_range(sector, extra_readable, limits.capacity_bytes)?;
            BlockOp::Write { offset, len }
        }
        VIRTIO_BLK_T_FLUSH if limits.flush => {
            if extra_readable != 0 || data_writable != 0 {
                return Err(RequestError::DirectionMismatch);
            }
            BlockOp::Flush
        }
        VIRTIO_BLK_T_GET_ID => {
            let max = u64::try_from(BLK_ID_LEN).unwrap_or(u64::MAX);
            if extra_readable != 0 || data_writable == 0 || data_writable > max {
                return Err(RequestError::DirectionMismatch);
            }
            BlockOp::GetId {
                len: u32::try_from(data_writable).unwrap_or(u32::MAX),
            }
        }
        other => return Err(RequestError::UnsupportedType { ty: other }),
    };
    Ok(BlockRequest { op, status_skip })
}

fn data_range(sector: u64, len: u64, capacity_bytes: u64) -> Result<(u64, u32), RequestError> {
    if len == 0 || !len.is_multiple_of(SECTOR_SIZE) {
        return Err(RequestError::Unaligned { len });
    }
    if len > MAX_REQUEST_BYTES {
        return Err(RequestError::TooLarge { len });
    }
    let offset = sector
        .checked_mul(SECTOR_SIZE)
        .ok_or(RequestError::SectorOverflow)?;
    let end = offset.checked_add(len).ok_or(RequestError::RangeOverflow)?;
    if end > capacity_bytes {
        return Err(RequestError::BeyondCapacity);
    }
    let len = u32::try_from(len).map_err(|_| RequestError::TooLarge { len })?;
    Ok((offset, len))
}
