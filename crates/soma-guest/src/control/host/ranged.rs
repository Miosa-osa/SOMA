//! Byte ranges of a file, and the guest API level probe.
//!
//! These are guest API level 2 operations. A host must learn the level with
//! [`RepairedHostControl::probe_guest_api`] before it sends one, because an older guest rejects
//! an unknown request tag as a protocol fault and ends the session.

use crate::{FileFailure, FileOutcome, FileRequest, MAX_CHUNK_BYTES};

use super::super::error::{ControlError, ControlFailureClass, ControlStage};
use super::whole_file::WholeFileWrite;
use super::{HostControlIo, RepairedHostControl};

/// The virtual directory a probe reads `<prefix><nonce>` under.
///
/// The agent answers from memory without touching the disk, so a file a tenant planted at this
/// path can never be mistaken for the answer: the answer must echo a nonce the guest saw for the
/// first time in the probe itself.
pub const GUEST_API_PROBE_PREFIX: &[u8] = b"/.soma/guest-api/";

/// Longest nonce a probe may carry.
pub const MAX_PROBE_NONCE_BYTES: usize = 64;

/// What a probe learned about the guest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestApi {
    /// The guest speaks only the original file protocol.
    Legacy,
    /// The guest echoed the nonce and named this level.
    Level(u8),
}

impl GuestApi {
    /// Whether this guest answers level 2 file requests: stat, rename, ranges, and chmod.
    #[must_use]
    pub const fn has_ranged_files(self) -> bool {
        matches!(self, Self::Level(level) if level >= 2)
    }
}

/// The result of reading a byte range.
#[derive(Clone, Eq, PartialEq)]
pub enum RangeRead {
    /// The bytes read, with whether the file ended within them.
    Bytes {
        /// At most the length asked for.
        bytes: Vec<u8>,
        /// Whether the read reached the end of the file.
        end: bool,
    },
    /// The range asked for more than the caller admitted, so nothing was sent.
    TooLarge,
    /// The guest did not perform the read.
    Failed(FileFailure),
}

impl core::fmt::Debug for RangeRead {
    /// Reports the shape and never the bytes, which are tenant data.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bytes { bytes, end } => {
                write!(formatter, "Bytes({} bytes, end: {end})", bytes.len())
            }
            Self::TooLarge => formatter.write_str("TooLarge"),
            Self::Failed(failure) => write!(formatter, "Failed({failure})"),
        }
    }
}

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Asks the guest which API level it speaks, using a read an older guest cannot mistake.
    ///
    /// The nonce must be fresh per session and alphanumeric. Only an exact `"<level> <nonce>\n"`
    /// answer counts; any other answer, including a plain file read or a failure, means the guest
    /// is [`GuestApi::Legacy`]. A legacy guest only ever sees an ordinary read request.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once.
    pub fn probe_guest_api(mut self, nonce: &[u8]) -> Result<(Self, GuestApi), ControlError> {
        let usable = !nonce.is_empty()
            && nonce.len() <= MAX_PROBE_NONCE_BYTES
            && nonce.iter().all(u8::is_ascii_alphanumeric);
        if !usable {
            return Ok((self, GuestApi::Legacy));
        }
        let mut path = GUEST_API_PROBE_PREFIX.to_vec();
        path.extend_from_slice(nonce);
        let length = u32::try_from(MAX_PROBE_NONCE_BYTES + 8).unwrap_or(u32::MAX);
        let (session, outcome) = self.file(FileRequest::Read {
            path: path.into(),
            offset: 0,
            length,
        })?;
        self = session;
        let api = match outcome {
            FileOutcome::Read { bytes, .. } => parse_probe(&bytes, nonce),
            _ => GuestApi::Legacy,
        };
        Ok((self, api))
    }

    /// Reads at most `length` bytes of a file from `offset`, refusing a range over `maximum`.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once, including when
    /// the guest returns more than it was asked for or no progress without the end.
    pub fn read_range(
        mut self,
        path: &[u8],
        offset: u64,
        length: usize,
        maximum: usize,
    ) -> Result<(Self, RangeRead), ControlError> {
        if length > maximum {
            return Ok((self, RangeRead::TooLarge));
        }
        let mut collected: Vec<u8> = Vec::new();
        let mut next = offset;
        loop {
            let wanted = (length - collected.len()).min(MAX_CHUNK_BYTES);
            if wanted == 0 {
                return Ok((
                    self,
                    RangeRead::Bytes {
                        bytes: collected,
                        end: false,
                    },
                ));
            }
            let (session, outcome) = self.file(FileRequest::Read {
                path: path.into(),
                offset: next,
                length: u32::try_from(wanted).unwrap_or(u32::MAX),
            })?;
            self = session;
            let (bytes, end) = match outcome {
                FileOutcome::Read { bytes, end } => (bytes, end),
                FileOutcome::Failed(failure) => return Ok((self, RangeRead::Failed(failure))),
                _ => {
                    return Err(self
                        .channel
                        .fail(ControlStage::File, ControlFailureClass::Protocol));
                }
            };
            if bytes.len() > wanted || (bytes.is_empty() && !end) {
                return Err(self
                    .channel
                    .fail(ControlStage::File, ControlFailureClass::Protocol));
            }
            next = next.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            collected.extend_from_slice(&bytes);
            if end {
                return Ok((
                    self,
                    RangeRead::Bytes {
                        bytes: collected,
                        end: true,
                    },
                ));
            }
        }
    }

    /// Writes `bytes` into a file at `offset`, refusing more than `maximum` bytes.
    ///
    /// Only the first record may create the file and only the last may shorten it, so a range
    /// write that does not ask to shorten never cuts off what follows it.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once, including when
    /// the guest answers a write with anything but the exact count of bytes it was handed.
    pub fn write_range(
        mut self,
        path: &[u8],
        offset: u64,
        create: bool,
        shorten: bool,
        bytes: &[u8],
        maximum: usize,
    ) -> Result<(Self, WholeFileWrite), ControlError> {
        if bytes.len() > maximum {
            return Ok((self, WholeFileWrite::TooLarge));
        }
        let mut position = offset;
        let mut remaining = bytes;
        let mut first = true;
        loop {
            let (chunk, rest) = remaining.split_at(remaining.len().min(MAX_CHUNK_BYTES));
            let last = rest.is_empty();
            let expected = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
            let (session, outcome) = self.file(FileRequest::Write {
                path: path.into(),
                offset: position,
                create: create && first,
                shorten: shorten && last,
                bytes: chunk.into(),
            })?;
            self = session;
            match outcome {
                FileOutcome::Written { bytes: written } if written == expected => {}
                FileOutcome::Failed(failure) => {
                    return Ok((self, WholeFileWrite::Failed(failure)));
                }
                _ => {
                    return Err(self
                        .channel
                        .fail(ControlStage::File, ControlFailureClass::Protocol));
                }
            }
            if last {
                return Ok((self, WholeFileWrite::Written));
            }
            first = false;
            position = position.saturating_add(expected);
            remaining = rest;
        }
    }
}

/// Accepts exactly `"<level> <nonce>\n"` with a one-digit level, and nothing else.
fn parse_probe(bytes: &[u8], nonce: &[u8]) -> GuestApi {
    let [level, b' ', rest @ ..] = bytes else {
        return GuestApi::Legacy;
    };
    let Some(echo) = rest.strip_suffix(b"\n") else {
        return GuestApi::Legacy;
    };
    if echo == nonce && level.is_ascii_digit() {
        GuestApi::Level(level - b'0')
    } else {
        GuestApi::Legacy
    }
}
