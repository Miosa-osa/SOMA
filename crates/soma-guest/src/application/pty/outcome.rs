//! What the guest answers one terminal request with.
//!
//! A failure is a small closed set of causes rather than an errno or a message, for the same
//! reason a filesystem failure is: an errno describes the guest's implementation to the host,
//! and a message can carry tenant data into whatever recorded it.

use core::fmt;

use crate::Error;

use super::super::frame::Reader;
use super::{MAX_PTY_CHUNK_BYTES, PtySize};

/// Why a terminal request did not succeed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PtyFailure {
    /// No terminal session is open, so there is nothing to write to, read, resize, or close.
    NoSession = 1,
    /// A session is already open, and this protocol carries exactly one at a time.
    AlreadyOpen = 2,
    /// The guest refused to start a terminal at all.
    Denied = 3,
    /// The guest could not complete the operation for any other reason.
    Failed = 4,
}

impl PtyFailure {
    /// Decodes one failure code.
    fn parse(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::NoSession),
            2 => Ok(Self::AlreadyOpen),
            3 => Ok(Self::Denied),
            4 => Ok(Self::Failed),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

impl fmt::Display for PtyFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoSession => "no terminal session",
            Self::AlreadyOpen => "a terminal session is already open",
            Self::Denied => "denied",
            Self::Failed => "failed",
        })
    }
}

/// The guest's answer to one terminal request.
#[derive(Clone, Eq, PartialEq)]
pub enum PtyOutcome {
    /// The session is open at these dimensions.
    Opened(PtySize),
    /// The bytes the terminal accepted, which may be fewer than the request offered.
    Wrote {
        /// How many leading bytes of the request were written.
        bytes: u32,
    },
    /// One bounded chunk of terminal output, with whether the session has ended.
    Output {
        /// The bytes, at most [`MAX_PTY_CHUNK_BYTES`] and possibly none.
        bytes: Box<[u8]>,
        /// Whether the session has ended and no further byte will ever follow.
        ///
        /// This is the explicit end of the stream. Output is unbounded in principle while one
        /// record is bounded in fact, so a caller drains a terminal by reading until this is
        /// set rather than by guessing from an empty chunk, which only means nothing arrived
        /// within the wait.
        end: bool,
    },
    /// The terminal was told its new dimensions.
    Resized(PtySize),
    /// The session and everything running under it are gone.
    Closed,
    /// The operation did not happen.
    Failed(PtyFailure),
}

impl fmt::Debug for PtyOutcome {
    /// Reports shapes and counts, and never a byte the terminal produced.
    ///
    /// Terminal output is tenant data down to the prompt it prints, so none of it may reach a
    /// log through a formatter.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Opened(size) => write!(formatter, "Opened({size:?})"),
            Self::Wrote { bytes } => write!(formatter, "Wrote {{ {bytes} }}"),
            Self::Output { bytes, end } => {
                write!(formatter, "Output {{ {} bytes, end: {end} }}", bytes.len())
            }
            Self::Resized(size) => write!(formatter, "Resized({size:?})"),
            Self::Closed => formatter.write_str("Closed"),
            Self::Failed(failure) => write!(formatter, "Failed({failure})"),
        }
    }
}

const OPENED: u8 = 1;
const WROTE: u8 = 2;
const OUTPUT: u8 = 3;
const RESIZED: u8 = 4;
const CLOSED: u8 = 5;
const FAILED: u8 = 6;

impl PtyOutcome {
    /// Encodes this outcome as one frame body.
    #[must_use]
    pub(in super::super) fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8);
        match self {
            Self::Opened(size) => {
                out.push(OPENED);
                size.put(&mut out);
            }
            Self::Wrote { bytes } => {
                out.push(WROTE);
                out.extend_from_slice(&bytes.to_be_bytes());
            }
            Self::Output { bytes, end } => {
                out.push(OUTPUT);
                out.push(u8::from(*end));
                put_field(&mut out, bytes);
            }
            Self::Resized(size) => {
                out.push(RESIZED);
                size.put(&mut out);
            }
            Self::Closed => out.push(CLOSED),
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
            OPENED => Self::Opened(PtySize::read(&mut reader)?),
            WROTE => Self::Wrote {
                bytes: reader.u32()?,
            },
            OUTPUT => {
                let end = flag(&mut reader)?;
                let bytes = reader.field(MAX_PTY_CHUNK_BYTES)?.into();
                Self::Output { bytes, end }
            }
            RESIZED => Self::Resized(PtySize::read(&mut reader)?),
            CLOSED => Self::Closed,
            FAILED => Self::Failed(PtyFailure::parse(reader.u8()?)?),
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
