//! The exact bytes of one filesystem request body.
//!
//! Each body starts with the path as a length-prefixed field so a decoder rejects an
//! inadmissible path before it reads anything that depends on it, and every remaining field is
//! fixed width. Booleans are carried as a single byte that accepts only zero or one, because a
//! decoder that treated any non-zero byte as true would accept several encodings of one message
//! and make the wire form ambiguous.

use crate::Error;

use super::super::frame::Reader;
use super::{FileRequest, MAX_CHUNK_BYTES, check_path};

/// Discriminants, chosen once and never reused.
const READ: u8 = 1;
const WRITE: u8 = 2;
const MAKE_DIRECTORY: u8 = 3;
const READ_DIRECTORY: u8 = 4;
const EXISTS: u8 = 5;
const REMOVE: u8 = 6;
const CREATE: u8 = 7;
const SET_MODE: u8 = 8;

impl FileRequest {
    /// Encodes this request as one frame body.
    #[must_use]
    pub(in super::super) fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.path().len() + 32);
        match self {
            Self::Read {
                path,
                offset,
                length,
            } => {
                out.push(READ);
                put_field(&mut out, path);
                out.extend_from_slice(&offset.to_be_bytes());
                out.extend_from_slice(&length.to_be_bytes());
            }
            Self::Write {
                path,
                offset,
                create,
                shorten,
                bytes,
            } => {
                out.push(WRITE);
                put_field(&mut out, path);
                out.extend_from_slice(&offset.to_be_bytes());
                out.push(u8::from(*create));
                out.push(u8::from(*shorten));
                put_field(&mut out, bytes);
            }
            Self::MakeDirectory { path, parents } => {
                out.push(MAKE_DIRECTORY);
                put_field(&mut out, path);
                out.push(u8::from(*parents));
            }
            Self::ReadDirectory { path, offset } => {
                out.push(READ_DIRECTORY);
                put_field(&mut out, path);
                out.extend_from_slice(&offset.to_be_bytes());
            }
            Self::Exists { path } => {
                out.push(EXISTS);
                put_field(&mut out, path);
            }
            Self::Remove { path, recursive } => {
                out.push(REMOVE);
                put_field(&mut out, path);
                out.push(u8::from(*recursive));
            }
            Self::Create { path, mode } => {
                out.push(CREATE);
                put_field(&mut out, path);
                out.extend_from_slice(&mode.to_be_bytes());
            }
            Self::SetMode { path, mode } => {
                out.push(SET_MODE);
                put_field(&mut out, path);
                out.extend_from_slice(&mode.to_be_bytes());
            }
        }
        out
    }

    /// Decodes one exact request body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ApplicationMessageRejected`] for every malformed body, including a
    /// trailing byte, an inadmissible path, an oversized payload, and a boolean that is neither
    /// zero nor one.
    pub(in super::super) fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(body);
        let request = match reader.u8()? {
            READ => {
                let path = super::read_path(&mut reader)?.into();
                let offset = reader.u64()?;
                let length = reader.u32()?;
                if usize::try_from(length).is_ok_and(|length| length > MAX_CHUNK_BYTES) {
                    return Err(Error::ApplicationMessageRejected);
                }
                Self::Read {
                    path,
                    offset,
                    length,
                }
            }
            WRITE => {
                let path = super::read_path(&mut reader)?.into();
                let offset = reader.u64()?;
                let create = flag(&mut reader)?;
                let shorten = flag(&mut reader)?;
                let bytes = reader.field(MAX_CHUNK_BYTES)?.into();
                Self::Write {
                    path,
                    offset,
                    create,
                    shorten,
                    bytes,
                }
            }
            MAKE_DIRECTORY => Self::MakeDirectory {
                path: super::read_path(&mut reader)?.into(),
                parents: flag(&mut reader)?,
            },
            READ_DIRECTORY => Self::ReadDirectory {
                path: super::read_path(&mut reader)?.into(),
                offset: reader.u32()?,
            },
            EXISTS => Self::Exists {
                path: super::read_path(&mut reader)?.into(),
            },
            REMOVE => Self::Remove {
                path: super::read_path(&mut reader)?.into(),
                recursive: flag(&mut reader)?,
            },
            CREATE => Self::Create {
                path: super::read_path(&mut reader)?.into(),
                mode: read_mode(&mut reader)?,
            },
            SET_MODE => Self::SetMode {
                path: super::read_path(&mut reader)?.into(),
                mode: read_mode(&mut reader)?,
            },
            _ => return Err(Error::ApplicationMessageRejected),
        };
        reader.finish()?;
        // An encoder cannot produce an inadmissible path, so this only ever rejects a body built
        // elsewhere; checking it again costs nothing and keeps the invariant local.
        check_path(request.path())?;
        Ok(request)
    }
}

/// Reads one permission field and rejects a value this protocol will not carry.
fn read_mode(reader: &mut Reader<'_>) -> Result<u32, Error> {
    let mode = reader.u32()?;
    super::check_mode(mode)?;
    Ok(mode)
}

/// Reads one byte that must be exactly zero or one.
fn flag(reader: &mut Reader<'_>) -> Result<bool, Error> {
    match reader.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::ApplicationMessageRejected),
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
