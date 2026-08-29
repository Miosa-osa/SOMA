//! The 44-byte `virtio_vsock_hdr` and guest-to-host packet validation.
//!
//! Every field is checked against the fixed SOMA contract before any byte of
//! payload is copied: stream type only, an allowlisted operation, the
//! assigned guest CID as source, the host CID as destination, an exact
//! declared length, a bounded payload, and flags only on `SHUTDOWN`.

use std::fmt;

use crate::virtio::devices::segments::read_readable;
use crate::virtio::guest_memory::{GuestMemory, GuestMemoryError};
use crate::virtio::queue::chain::DescriptorChain;

/// Virtio device identifier for a socket device.
pub const VIRTIO_VSOCK_DEVICE_ID: u32 = 19;
/// The well-known host context identifier.
pub const HOST_CID: u64 = 2;
/// The fixed SOMA control port the host endpoint accepts: `"SOMA"` as ASCII.
///
/// This is the same value as `soma_guest::CONTROL_VSOCK_PORT`; the two
/// constants are one machine-contract field and must change together.
/// `soma-kvm` restates the literal rather than depending on `soma-guest`.
pub const SOMA_CONTROL_PORT: u32 = 0x534f_4d41;
/// Header length.
pub const VSOCK_HDR_LEN: usize = 44;
/// Largest payload one packet may carry in either direction.
pub const MAX_PAYLOAD_LEN: u32 = 1 << 16;
/// Socket type: stream.
pub const VSOCK_TYPE_STREAM: u16 = 1;
pub const VSOCK_OP_INVALID: u16 = 0;
pub const VSOCK_OP_REQUEST: u16 = 1;
pub const VSOCK_OP_RESPONSE: u16 = 2;
pub const VSOCK_OP_RST: u16 = 3;
pub const VSOCK_OP_SHUTDOWN: u16 = 4;
pub const VSOCK_OP_RW: u16 = 5;
pub const VSOCK_OP_CREDIT_UPDATE: u16 = 6;
pub const VSOCK_OP_CREDIT_REQUEST: u16 = 7;
/// Shutdown flag: the peer will not receive more.
pub const VSOCK_SHUTDOWN_RCV: u32 = 1;
/// Shutdown flag: the peer will not send more.
pub const VSOCK_SHUTDOWN_SEND: u32 = 2;
/// Event: all connections were lost; sent after restore.
pub const VSOCK_EVENT_TRANSPORT_RESET: u32 = 0;

/// One packet header in host byte order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VsockHeader {
    pub src_cid: u64,
    pub dst_cid: u64,
    pub src_port: u32,
    pub dst_port: u32,
    pub len: u32,
    pub ty: u16,
    pub op: u16,
    pub flags: u32,
    pub buf_alloc: u32,
    pub fwd_cnt: u32,
}

impl VsockHeader {
    /// Encodes in the guest little-endian layout.
    #[must_use]
    pub fn to_bytes(self) -> [u8; VSOCK_HDR_LEN] {
        let mut raw = [0u8; VSOCK_HDR_LEN];
        raw[0..8].copy_from_slice(&self.src_cid.to_le_bytes());
        raw[8..16].copy_from_slice(&self.dst_cid.to_le_bytes());
        raw[16..20].copy_from_slice(&self.src_port.to_le_bytes());
        raw[20..24].copy_from_slice(&self.dst_port.to_le_bytes());
        raw[24..28].copy_from_slice(&self.len.to_le_bytes());
        raw[28..30].copy_from_slice(&self.ty.to_le_bytes());
        raw[30..32].copy_from_slice(&self.op.to_le_bytes());
        raw[32..36].copy_from_slice(&self.flags.to_le_bytes());
        raw[36..40].copy_from_slice(&self.buf_alloc.to_le_bytes());
        raw[40..44].copy_from_slice(&self.fwd_cnt.to_le_bytes());
        raw
    }

    /// Decodes from the guest little-endian layout.
    #[must_use]
    pub fn from_bytes(raw: &[u8; VSOCK_HDR_LEN]) -> Self {
        let u64_at = |o: usize| u64::from_le_bytes(raw[o..o + 8].try_into().unwrap_or([0; 8]));
        let u32_at = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap_or([0; 4]));
        let u16_at = |o: usize| u16::from_le_bytes([raw[o], raw[o + 1]]);
        Self {
            src_cid: u64_at(0),
            dst_cid: u64_at(8),
            src_port: u32_at(16),
            dst_port: u32_at(20),
            len: u32_at(24),
            ty: u16_at(28),
            op: u16_at(30),
            flags: u32_at(32),
            buf_alloc: u32_at(36),
            fwd_cnt: u32_at(40),
        }
    }
}

/// Why a guest packet was rejected; carries field values only, never payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketError {
    /// A transmit chain must be device-readable only.
    WritableSegment,
    /// Fewer than 44 readable bytes.
    NoHeader { len: u64 },
    /// The validated header could not be read.
    Memory(GuestMemoryError),
    /// Not a stream socket.
    Type { ty: u16 },
    /// Not an allowlisted operation.
    Op { op: u16 },
    /// Source is not the assigned guest CID.
    SourceCid { cid: u64 },
    /// Destination is not the host CID.
    DestinationCid { cid: u64 },
    /// Declared length differs from the readable payload.
    LengthMismatch { declared: u32, actual: u64 },
    /// Payload above [`MAX_PAYLOAD_LEN`].
    PayloadTooLarge { len: u32 },
    /// A non-`RW` packet carries payload.
    PayloadOnControl { op: u16 },
    /// Flags are set on a packet that must not carry them, or shutdown flags are invalid.
    Flags { op: u16, flags: u32 },
}

impl fmt::Display for PacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "vsock packet rejected: {self:?}")
    }
}

impl std::error::Error for PacketError {}

const ALLOWED_OPS: [u16; 7] = [
    VSOCK_OP_REQUEST,
    VSOCK_OP_RESPONSE,
    VSOCK_OP_RST,
    VSOCK_OP_SHUTDOWN,
    VSOCK_OP_RW,
    VSOCK_OP_CREDIT_UPDATE,
    VSOCK_OP_CREDIT_REQUEST,
];

/// Validates a transmit chain and returns its header; the payload stays in
/// guest memory at readable offset 44 and is exactly `header.len` bytes.
///
/// The destination port is not checked here so the device can answer a
/// `REQUEST` to a closed port with `RST` as the specification requires.
///
/// # Errors
/// Returns the first typed rejection in check order.
pub fn parse_tx<M: GuestMemory + ?Sized>(
    mem: &M,
    chain: &DescriptorChain,
    guest_cid: u64,
) -> Result<VsockHeader, PacketError> {
    if chain.writable_len() != 0 {
        return Err(PacketError::WritableSegment);
    }
    let readable = chain.readable_len();
    let hdr_len = u64::try_from(VSOCK_HDR_LEN).unwrap_or(u64::MAX);
    if readable < hdr_len {
        return Err(PacketError::NoHeader { len: readable });
    }
    let mut raw = [0u8; VSOCK_HDR_LEN];
    read_readable(mem, chain, 0, &mut raw).map_err(PacketError::Memory)?;
    let header = VsockHeader::from_bytes(&raw);
    if header.ty != VSOCK_TYPE_STREAM {
        return Err(PacketError::Type { ty: header.ty });
    }
    if !ALLOWED_OPS.contains(&header.op) {
        return Err(PacketError::Op { op: header.op });
    }
    if header.src_cid != guest_cid {
        return Err(PacketError::SourceCid {
            cid: header.src_cid,
        });
    }
    if header.dst_cid != HOST_CID {
        return Err(PacketError::DestinationCid {
            cid: header.dst_cid,
        });
    }
    let actual = readable - hdr_len;
    if u64::from(header.len) != actual {
        return Err(PacketError::LengthMismatch {
            declared: header.len,
            actual,
        });
    }
    if header.len > MAX_PAYLOAD_LEN {
        return Err(PacketError::PayloadTooLarge { len: header.len });
    }
    if header.op != VSOCK_OP_RW && header.len != 0 {
        return Err(PacketError::PayloadOnControl { op: header.op });
    }
    let flags_ok = match header.op {
        VSOCK_OP_SHUTDOWN => {
            header.flags != 0 && header.flags & !(VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND) == 0
        }
        _ => header.flags == 0,
    };
    if !flags_ok {
        return Err(PacketError::Flags {
            op: header.op,
            flags: header.flags,
        });
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_port_is_the_machine_contract_value_shared_with_the_guest() {
        assert_eq!(SOMA_CONTROL_PORT, 0x534f_4d41);
        assert_eq!(SOMA_CONTROL_PORT.to_be_bytes(), *b"SOMA");
    }

    #[test]
    fn header_encoding_round_trips_every_field() {
        let header = VsockHeader {
            src_cid: 0x0102_0304_0506_0708,
            dst_cid: HOST_CID,
            src_port: 0x1111_2222,
            dst_port: SOMA_CONTROL_PORT,
            len: 7,
            ty: VSOCK_TYPE_STREAM,
            op: VSOCK_OP_RW,
            flags: 3,
            buf_alloc: 0xabcd,
            fwd_cnt: 0xef01,
        };
        let raw = header.to_bytes();
        assert_eq!(&raw[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(VsockHeader::from_bytes(&raw), header);
    }
}
