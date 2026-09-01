//! The exact bytes of one terminal request body.
//!
//! Every body is a discriminant followed by fixed-width fields, and the one variable field is
//! length prefixed with the same `u16` prefix every field on this protocol uses. Dimensions are
//! decoded through [`PtySize::new`], so a body naming a zero or an oversized terminal is refused
//! here rather than accepted and clamped somewhere further in; clamping would give two different
//! bodies the same meaning.

use crate::Error;

use super::super::frame::Reader;
use super::{MAX_PTY_CHUNK_BYTES, MAX_PTY_WAIT_MILLIS, PtyRequest, PtySize};

/// Discriminants, chosen once and never reused.
const OPEN: u8 = 1;
const WRITE: u8 = 2;
const READ: u8 = 3;
const RESIZE: u8 = 4;
const CLOSE: u8 = 5;

impl PtySize {
    /// Appends the two dimensions in the order they are read back.
    pub(super) fn put(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.columns().to_be_bytes());
        out.extend_from_slice(&self.rows().to_be_bytes());
    }

    /// Reads two dimensions and refuses a terminal this protocol will not carry.
    pub(super) fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
        let columns = reader.u16()?;
        let rows = reader.u16()?;
        Self::new(columns, rows).map_err(|_| Error::ApplicationMessageRejected)
    }
}

impl PtyRequest {
    /// Encodes this request as one frame body.
    ///
    /// Public for the reason the filesystem request's is: the portable facade restates this
    /// protocol's terminal bounds so a surface can refuse an inadmissible call before the wire,
    /// and the only honest way to hold the two equal is to run a candidate through the encode and
    /// decode a real call takes.
    #[must_use]
    pub fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8);
        match self {
            Self::Open(size) => {
                out.push(OPEN);
                size.put(&mut out);
            }
            Self::Write { bytes } => {
                out.push(WRITE);
                put_field(&mut out, bytes);
            }
            Self::Read { wait_millis } => {
                out.push(READ);
                out.extend_from_slice(&wait_millis.to_be_bytes());
            }
            Self::Resize(size) => {
                out.push(RESIZE);
                size.put(&mut out);
            }
            Self::Close => out.push(CLOSE),
        }
        out
    }

    /// Decodes one exact request body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ApplicationMessageRejected`] for every malformed body, including a
    /// trailing byte, an inadmissible terminal size, an oversized chunk, and a wait beyond
    /// [`MAX_PTY_WAIT_MILLIS`].
    pub fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(body);
        let request = match reader.u8()? {
            OPEN => Self::Open(PtySize::read(&mut reader)?),
            WRITE => Self::Write {
                bytes: reader.field(MAX_PTY_CHUNK_BYTES)?.into(),
            },
            READ => {
                let wait_millis = reader.u32()?;
                if wait_millis > MAX_PTY_WAIT_MILLIS {
                    return Err(Error::ApplicationMessageRejected);
                }
                Self::Read { wait_millis }
            }
            RESIZE => Self::Resize(PtySize::read(&mut reader)?),
            CLOSE => Self::Close,
            _ => return Err(Error::ApplicationMessageRejected),
        };
        reader.finish()?;
        Ok(request)
    }
}

/// Writes one length-prefixed field, using the same `u16` prefix every field on this
/// protocol uses.
///
/// Every caller writes a field this protocol has already bounded below `u16::MAX`, so a longer
/// one is a caller bug rather than a wire condition, and clamping would silently shorten the
/// field instead of failing.
fn put_field(out: &mut Vec<u8>, bytes: &[u8]) {
    let length = u16::try_from(bytes.len()).expect("a bounded field length");
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
}
