//! Fixed little-endian request and response records for the virtio-mem queue.
//!
//! A request is 24 bytes (`type`, 3 padding words, `addr`, `nb_blocks`, 3
//! padding words) and a response is 10 bytes (`type`, 3 padding words,
//! `state`). Padding is ignored on input and zero on output.

/// Encoded request length.
pub const REQUEST_LEN: usize = 24;
/// Encoded response length.
pub const RESPONSE_LEN: usize = 10;

/// Request type: plug a block range.
pub const VIRTIO_MEM_REQ_PLUG: u16 = 0;
/// Request type: unplug a block range.
pub const VIRTIO_MEM_REQ_UNPLUG: u16 = 1;
/// Request type: unplug every block.
pub const VIRTIO_MEM_REQ_UNPLUG_ALL: u16 = 2;
/// Request type: query the state of a block range.
pub const VIRTIO_MEM_REQ_STATE: u16 = 3;

/// Response type: request done.
pub const VIRTIO_MEM_RESP_ACK: u16 = 0;
/// Response type: request refused, retry later or smaller.
pub const VIRTIO_MEM_RESP_NACK: u16 = 1;
/// Response type: host busy, retry later.
pub const VIRTIO_MEM_RESP_BUSY: u16 = 2;
/// Response type: invalid request.
pub const VIRTIO_MEM_RESP_ERROR: u16 = 3;

/// State value: every block in the range is plugged.
pub const VIRTIO_MEM_STATE_PLUGGED: u16 = 0;
/// State value: no block in the range is plugged.
pub const VIRTIO_MEM_STATE_UNPLUGGED: u16 = 1;
/// State value: the range is partly plugged.
pub const VIRTIO_MEM_STATE_MIXED: u16 = 2;

/// A request kind the device understands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestKind {
    Plug,
    Unplug,
    UnplugAll,
    State,
}

/// A decoded request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemRequest {
    pub kind: RequestKind,
    pub addr: u64,
    pub nb_blocks: u16,
}

/// A request header with an unknown type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownRequest {
    pub kind: u16,
}

impl MemRequest {
    /// Decodes a request.
    ///
    /// # Errors
    /// Returns the unknown type code; the caller answers `ERROR`.
    pub fn parse(raw: &[u8; REQUEST_LEN]) -> Result<Self, UnknownRequest> {
        let kind = u16::from_le_bytes([raw[0], raw[1]]);
        let addr = u64::from_le_bytes([
            raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15],
        ]);
        let nb_blocks = u16::from_le_bytes([raw[16], raw[17]]);
        let kind = match kind {
            VIRTIO_MEM_REQ_PLUG => RequestKind::Plug,
            VIRTIO_MEM_REQ_UNPLUG => RequestKind::Unplug,
            VIRTIO_MEM_REQ_UNPLUG_ALL => RequestKind::UnplugAll,
            VIRTIO_MEM_REQ_STATE => RequestKind::State,
            kind => return Err(UnknownRequest { kind }),
        };
        Ok(Self {
            kind,
            addr,
            nb_blocks,
        })
    }

    /// Encodes a request; used by the model driver in tests.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; REQUEST_LEN] {
        let kind = match self.kind {
            RequestKind::Plug => VIRTIO_MEM_REQ_PLUG,
            RequestKind::Unplug => VIRTIO_MEM_REQ_UNPLUG,
            RequestKind::UnplugAll => VIRTIO_MEM_REQ_UNPLUG_ALL,
            RequestKind::State => VIRTIO_MEM_REQ_STATE,
        };
        let mut raw = [0u8; REQUEST_LEN];
        raw[0..2].copy_from_slice(&kind.to_le_bytes());
        raw[8..16].copy_from_slice(&self.addr.to_le_bytes());
        raw[16..18].copy_from_slice(&self.nb_blocks.to_le_bytes());
        raw
    }
}

/// Encodes a response with an optional `STATE` payload (zero otherwise).
#[must_use]
pub fn encode_response(kind: u16, state: u16) -> [u8; RESPONSE_LEN] {
    let mut raw = [0u8; RESPONSE_LEN];
    raw[0..2].copy_from_slice(&kind.to_le_bytes());
    raw[8..10].copy_from_slice(&state.to_le_bytes());
    raw
}
