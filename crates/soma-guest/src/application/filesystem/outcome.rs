//! What the guest answers one filesystem request with.
//!
//! A failure is a small closed set of causes rather than an errno or a message. An errno would
//! leak the guest's implementation to the host, and a message would leak tenant data into
//! whatever recorded it, while a caller only ever needs to know which of these happened.

use core::fmt;

use crate::Error;

use super::super::frame::Reader;
use super::MAX_ENTRIES;

/// Why a filesystem request did not succeed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FileFailure {
    /// No such path.
    NotFound = 1,
    /// The guest refused the access.
    Denied = 2,
    /// A directory was named where a file was required, or the reverse.
    WrongKind = 3,
    /// The path already exists and the request required that it did not.
    Exists = 4,
    /// A directory was not empty and the request did not ask to remove its contents.
    NotEmpty = 5,
    /// The guest could not complete the operation for any other reason.
    Failed = 6,
}

impl FileFailure {
    /// Decodes one failure code.
    fn parse(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::NotFound),
            2 => Ok(Self::Denied),
            3 => Ok(Self::WrongKind),
            4 => Ok(Self::Exists),
            5 => Ok(Self::NotEmpty),
            6 => Ok(Self::Failed),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

impl fmt::Display for FileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NotFound => "not found",
            Self::Denied => "denied",
            Self::WrongKind => "wrong kind",
            Self::Exists => "already exists",
            Self::NotEmpty => "not empty",
            Self::Failed => "failed",
        };
        formatter.write_str(text)
    }
}

/// What one path is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EntryKind {
    /// A regular file.
    File = 1,
    /// A directory.
    Directory = 2,
    /// Anything else the guest reports, including a symbolic link or a device.
    Other = 3,
}

impl EntryKind {
    /// Decodes one entry kind.
    fn parse(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::File),
            2 => Ok(Self::Directory),
            3 => Ok(Self::Other),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

/// One directory entry: its name within the directory, never a path.
#[derive(Clone, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// The entry's own name, with no separator in it.
    pub name: Box<[u8]>,
    /// What the entry is.
    pub kind: EntryKind,
}

impl fmt::Debug for DirectoryEntry {
    /// Names the kind and the name length, never the name, which is tenant data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DirectoryEntry {{ kind: {:?}, name: {} bytes }}",
            self.kind,
            self.name.len()
        )
    }
}

/// The guest's answer to one filesystem request.
#[derive(Clone, Eq, PartialEq)]
pub enum FileOutcome {
    /// Bytes read, with whether the file ended within this read.
    Read {
        /// The bytes, which may be shorter than the request asked for.
        bytes: Box<[u8]>,
        /// Whether the read reached the end of the file.
        end: bool,
    },
    /// Bytes written.
    Written {
        /// How many bytes were written.
        bytes: u64,
    },
    /// The directory listing, with whether more entries remain beyond it.
    Listed {
        /// The entries, at most [`MAX_ENTRIES`] of them.
        entries: Vec<DirectoryEntry>,
        /// Whether the directory held more entries than this listing carries.
        more: bool,
    },
    /// What the named path is, or that it is absent.
    Status {
        /// The kind, or `None` when nothing exists at the path.
        kind: Option<EntryKind>,
    },
    /// The operation completed and has nothing to report.
    Done,
    /// The operation did not happen.
    Failed(FileFailure),
}

impl fmt::Debug for FileOutcome {
    /// Reports shapes and never bytes or names.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { bytes, end } => {
                write!(formatter, "Read {{ {} bytes, end: {end} }}", bytes.len())
            }
            Self::Written { bytes } => write!(formatter, "Written {{ {bytes} }}"),
            Self::Listed { entries, more } => {
                write!(
                    formatter,
                    "Listed {{ {} entries, more: {more} }}",
                    entries.len()
                )
            }
            Self::Status { kind } => write!(formatter, "Status {{ {kind:?} }}"),
            Self::Done => formatter.write_str("Done"),
            Self::Failed(failure) => write!(formatter, "Failed({failure})"),
        }
    }
}

const READ: u8 = 1;
const WRITTEN: u8 = 2;
const LISTED: u8 = 3;
const STATUS: u8 = 4;
const DONE: u8 = 5;
const FAILED: u8 = 6;

impl FileOutcome {
    /// Encodes this outcome as one frame body.
    #[must_use]
    pub(in super::super) fn encode_body(&self) -> Vec<u8> {
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
            Self::Status { kind } => {
                out.push(STATUS);
                out.push(kind.map_or(0, |kind| kind as u8));
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
    pub(in super::super) fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(body);
        let outcome = match reader.u8()? {
            READ => {
                let end = flag(&mut reader)?;
                let bytes = reader.field(super::MAX_CHUNK_BYTES)?.into();
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
                    let name: Box<[u8]> = reader.field(super::MAX_PATH_BYTES)?.into();
                    // A name is one component: empty or separator-bearing names would let a
                    // listing describe a path outside the directory it claims to describe.
                    if name.is_empty() || name.contains(&b'/') || name.contains(&0) {
                        return Err(Error::ApplicationMessageRejected);
                    }
                    entries.push(DirectoryEntry { name, kind });
                }
                Self::Listed { entries, more }
            }
            STATUS => Self::Status {
                kind: match reader.u8()? {
                    0 => None,
                    value => Some(EntryKind::parse(value)?),
                },
            },
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
