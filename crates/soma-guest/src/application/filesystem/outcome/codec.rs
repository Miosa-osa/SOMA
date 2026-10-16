//! The exact bytes of one filesystem outcome body.
//!
//! Every body starts with its discriminant. The two byte-string fields carry the same `u16`
//! length prefix every field on this protocol uses, and every remaining field is fixed width.
//! A listing entry name is checked to be one path component, because a name carrying a
//! separator would let a guest describe entries outside the directory it claims to describe.

use crate::Error;

use super::super::super::frame::Reader;
use super::super::{MAX_CHUNK_BYTES, MAX_ENTRIES, MAX_PATH_BYTES, check_mode};
use super::{DetailedEntry, DirectoryEntry, EntryKind, FileFailure, FileOutcome};

const READ: u8 = 1;
const WRITTEN: u8 = 2;
const LISTED: u8 = 3;
const STATUS: u8 = 4;
const DONE: u8 = 5;
const FAILED: u8 = 6;
const STAT: u8 = 7;
const LISTED_DETAILED: u8 = 8;

impl FileOutcome {
    /// Encodes this outcome as one frame body.
    ///
    /// Public for the reason [`super::super::FileRequest::encode_body`] gives: a relayed answer must be
    /// the same bytes the record layer carried, not a second encoding of them.
    #[must_use]
    pub fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Read { bytes, end } => {
                out.push(READ);
                out.push(u8::from(*end));
                put_field(&mut out, bytes);
            }
            Self::Written { bytes } => {
                out.push(WRITTEN);
                out.extend_from_slice(&bytes.to_be_bytes());
            }
            Self::Listed { entries, more } => {
                out.push(LISTED);
                out.push(u8::from(*more));
                let count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
                out.extend_from_slice(&count.to_be_bytes());
                for entry in entries {
                    out.push(entry.kind as u8);
                    put_field(&mut out, &entry.name);
                }
            }
            Self::ListedDetailed { entries, more } => {
                out.push(LISTED_DETAILED);
                out.push(u8::from(*more));
                let count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
                out.extend_from_slice(&count.to_be_bytes());
                for entry in entries {
                    out.push(entry.kind as u8);
                    put_field(&mut out, &entry.name);
                    out.extend_from_slice(&entry.size.to_be_bytes());
                    out.extend_from_slice(&entry.mode.to_be_bytes());
                    out.extend_from_slice(&entry.modified.to_be_bytes());
                }
            }
            Self::Status { kind } => {
                out.push(STATUS);
                out.push(kind.map_or(0, |kind| kind as u8));
            }
            Self::Stat {
                kind,
                size,
                mode,
                modified,
            } => {
                out.push(STAT);
                out.push(*kind as u8);
                out.extend_from_slice(&size.to_be_bytes());
                out.extend_from_slice(&mode.to_be_bytes());
                out.extend_from_slice(&modified.to_be_bytes());
            }
            Self::Done => out.push(DONE),
            Self::Failed(failure) => {
                out.push(FAILED);
                out.push(*failure as u8);
            }
        }
        out
    }

    /// Decodes one exact outcome body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ApplicationMessageRejected`] for every malformed body.
    pub fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(body);
        let outcome = match reader.u8()? {
            READ => {
                let end = flag(&mut reader)?;
                let bytes = reader.field(MAX_CHUNK_BYTES)?.into();
                Self::Read { bytes, end }
            }
            WRITTEN => Self::Written {
                bytes: reader.u64()?,
            },
            LISTED => {
                let more = flag(&mut reader)?;
                let count = reader.u32()?;
                let count =
                    usize::try_from(count).map_err(|_| Error::ApplicationMessageRejected)?;
                if count > MAX_ENTRIES {
                    return Err(Error::ApplicationMessageRejected);
                }
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let kind = EntryKind::parse(reader.u8()?)?;
                    let name: Box<[u8]> = reader.field(MAX_PATH_BYTES)?.into();
                    // A name is one component: empty or separator-bearing names would let a
                    // listing describe a path outside the directory it claims to describe.
                    if name.is_empty() || name.contains(&b'/') || name.contains(&0) {
                        return Err(Error::ApplicationMessageRejected);
                    }
                    entries.push(DirectoryEntry { name, kind });
                }
                Self::Listed { entries, more }
            }
            LISTED_DETAILED => {
                let more = flag(&mut reader)?;
                let count = reader.u32()?;
                let count =
                    usize::try_from(count).map_err(|_| Error::ApplicationMessageRejected)?;
                if count > MAX_ENTRIES {
                    return Err(Error::ApplicationMessageRejected);
                }
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let kind = EntryKind::parse(reader.u8()?)?;
                    let name: Box<[u8]> = reader.field(MAX_PATH_BYTES)?.into();
                    if name.is_empty() || name.contains(&b'/') || name.contains(&0) {
                        return Err(Error::ApplicationMessageRejected);
                    }
                    let size = reader.u64()?;
                    let permission = reader.u32()?;
                    check_mode(permission)?;
                    let modified = reader.u64()?;
                    entries.push(DetailedEntry {
                        name,
                        kind,
                        size,
                        mode: permission,
                        modified,
                    });
                }
                Self::ListedDetailed { entries, more }
            }
            STATUS => Self::Status {
                kind: match reader.u8()? {
                    0 => None,
                    value => Some(EntryKind::parse(value)?),
                },
            },
            STAT => {
                let kind = EntryKind::parse(reader.u8()?)?;
                let size = reader.u64()?;
                let mode = reader.u32()?;
                check_mode(mode)?;
                Self::Stat {
                    kind,
                    size,
                    mode,
                    modified: reader.u64()?,
                }
            }
            DONE => Self::Done,
            FAILED => Self::Failed(FileFailure::parse(reader.u8()?)?),
            _ => return Err(Error::ApplicationMessageRejected),
        };
        reader.finish()?;
        Ok(outcome)
    }
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
