use std::{convert::TryInto, fmt};

pub(crate) const HEADER_LEN: usize = 64;
pub(crate) const MAX_PAYLOAD: usize = 64 * 1024;
const MAGIC: [u8; 4] = *b"SMAC";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Kind {
    Hello = 1,
    Request = 2,
    Stdout = 3,
    Stderr = 4,
    Terminal = 5,
}

impl TryFrom<u8> for Kind {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Request),
            3 => Ok(Self::Stdout),
            4 => Ok(Self::Stderr),
            5 => Ok(Self::Terminal),
            _ => Err("unknown frame kind"),
        }
    }
}

#[derive(Eq, PartialEq)]
pub(crate) struct Frame {
    pub(crate) kind: Kind,
    pub(crate) request_id: u64,
    pub(crate) sequence: u32,
    pub(crate) challenge: [u8; 32],
    pub(crate) payload: Vec<u8>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Frame")
            .field("kind", &self.kind)
            .field("request_id", &self.request_id)
            .field("sequence", &self.sequence)
            .field("payload_len", &self.payload.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn encode(frame: &Frame) -> Result<Vec<u8>, &'static str> {
    let payload_len = u32::try_from(frame.payload.len()).map_err(|_| "payload length overflow")?;
    if frame.payload.len() > MAX_PAYLOAD {
        return Err("payload exceeds protocol limit");
    }
    let mut bytes = vec![0; HEADER_LEN];
    bytes[0..4].copy_from_slice(&MAGIC);
    bytes[4] = VERSION;
    bytes[5] = u8::try_from(HEADER_LEN).map_err(|_| "header length overflow")?;
    bytes[6] = frame.kind as u8;
    bytes[12..20].copy_from_slice(&frame.request_id.to_be_bytes());
    bytes[20..24].copy_from_slice(&frame.sequence.to_be_bytes());
    bytes[24..28].copy_from_slice(&payload_len.to_be_bytes());
    bytes[28..60].copy_from_slice(&frame.challenge);
    bytes.extend_from_slice(&frame.payload);
    let checksum = crc32c(&bytes[..60], &frame.payload);
    bytes[60..64].copy_from_slice(&checksum.to_be_bytes());
    Ok(bytes)
}

pub(crate) struct Decoder {
    bytes: Vec<u8>,
    expected_len: Option<usize>,
    poisoned: bool,
}

impl Decoder {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            expected_len: None,
            poisoned: false,
        }
    }

    pub(crate) fn push(&mut self, byte: u8) -> Result<Option<Frame>, &'static str> {
        if self.poisoned {
            return Err("frame decoder is poisoned");
        }
        self.bytes.push(byte);
        if self.bytes.len() == HEADER_LEN {
            let payload_len = match validate_header(&self.bytes) {
                Ok(length) => length,
                Err(error) => {
                    self.bytes.clear();
                    self.poisoned = true;
                    return Err(error);
                }
            };
            self.expected_len = Some(
                HEADER_LEN
                    .checked_add(payload_len)
                    .ok_or("frame length overflow")?,
            );
        }
        let Some(expected_len) = self.expected_len else {
            return Ok(None);
        };
        if self.bytes.len() < expected_len {
            return Ok(None);
        }
        if self.bytes.len() != expected_len {
            return Err("frame exceeded declared length");
        }
        let bytes = std::mem::take(&mut self.bytes);
        self.expected_len = None;
        match decode_complete(&bytes) {
            Ok(frame) => Ok(Some(frame)),
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

fn validate_header(bytes: &[u8]) -> Result<usize, &'static str> {
    if bytes.len() < HEADER_LEN || bytes[0..4] != MAGIC {
        return Err("invalid frame magic");
    }
    if bytes[4] != VERSION || usize::from(bytes[5]) != HEADER_LEN {
        return Err("unsupported frame version or header length");
    }
    Kind::try_from(bytes[6])?;
    if bytes[7..12].iter().any(|byte| *byte != 0) {
        return Err("frame flags or reserved bytes are nonzero");
    }
    let payload_len = u32::from_be_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if payload_len > MAX_PAYLOAD {
        return Err("payload exceeds protocol limit");
    }
    Ok(payload_len)
}

fn decode_complete(bytes: &[u8]) -> Result<Frame, &'static str> {
    let payload_len = validate_header(bytes)?;
    let expected_crc = u32::from_be_bytes(bytes[60..64].try_into().unwrap());
    if crc32c(&bytes[..60], &bytes[HEADER_LEN..]) != expected_crc {
        return Err("frame CRC32C mismatch");
    }
    Ok(Frame {
        kind: Kind::try_from(bytes[6])?,
        request_id: u64::from_be_bytes(bytes[12..20].try_into().unwrap()),
        sequence: u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        challenge: bytes[28..60].try_into().unwrap(),
        payload: bytes[HEADER_LEN..HEADER_LEN + payload_len].to_vec(),
    })
}

fn crc32c(header: &[u8], payload: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in header.iter().chain(payload) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0x82f6_3b78 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests;
