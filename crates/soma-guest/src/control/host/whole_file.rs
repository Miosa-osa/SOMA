//! Whole files carried across several bounded filesystem requests.
//!
//! One record moves at most [`MAX_CHUNK_BYTES`], so a whole file is a loop over the single
//! request the session already exposes. The loop lives here rather than in every caller because
//! the parts that are easy to get wrong, refusing to grow a buffer without a bound and refusing
//! to spin when the guest reports no progress, have to be got right once.

mod outcome;

pub use outcome::{WholeFileRead, WholeFileWrite};

use crate::{FileOutcome, FileRequest, MAX_CHUNK_BYTES};

use super::super::error::{ControlError, ControlFailureClass, ControlStage};
use super::{HostControlIo, RepairedHostControl};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Reads a whole file, refusing to hold more than `maximum` bytes of it.
    ///
    /// The bound is explicit because the host cannot know what the guest will offer: a file the
    /// sandbox grew to any size would otherwise decide how much host memory this call takes.
    /// A file that does not end within the bound returns [`WholeFileRead::TooLarge`] and leaves
    /// the session usable, since nothing about the transport went wrong.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once, including when
    /// the guest answers a read with something other than read bytes or reports no progress.
    pub fn read_whole_file(
        mut self,
        path: &[u8],
        maximum: usize,
    ) -> Result<(Self, WholeFileRead), ControlError> {
        let mut collected: Vec<u8> = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let wanted = (maximum - collected.len()).min(MAX_CHUNK_BYTES);
            if wanted == 0 {
                // The guest has not reported the end of the file and the caller's bound admits
                // no further byte, so the file is larger than the caller agreed to hold.
                return Ok((self, WholeFileRead::TooLarge));
            }
            let request = FileRequest::Read {
                path: path.into(),
                offset,
                length: u32::try_from(wanted).unwrap_or(u32::MAX),
            };
            let (session, outcome) = self.file(request)?;
            self = session;
            let (bytes, end) = match outcome {
                FileOutcome::Read { bytes, end } => (bytes, end),
                FileOutcome::Failed(failure) => {
                    return Ok((self, WholeFileRead::Failed(failure)));
                }
                _ => {
                    return Err(self
                        .channel
                        .fail(ControlStage::File, ControlFailureClass::Protocol));
                }
            };
            // A guest that returns more than it was asked for, or that returns nothing while
            // claiming the file continues, would make this loop overrun the caller's bound or
            // never finish, so both answers end the session instead.
            if bytes.len() > wanted || (bytes.is_empty() && !end) {
                return Err(self
                    .channel
                    .fail(ControlStage::File, ControlFailureClass::Protocol));
            }
            offset = offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            collected.extend_from_slice(&bytes);
            if end {
                return Ok((self, WholeFileRead::Bytes(collected)));
            }
        }
    }

    /// Writes a whole file, replacing whatever was at the path, for at most `maximum` bytes.
    ///
    /// The bound is explicit here too so that one call cannot start a transfer of any length the
    /// caller did not mean to authorise; an oversized slice returns [`WholeFileWrite::TooLarge`]
    /// before anything reaches the wire.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once, including when
    /// the guest answers a write with anything but the exact count of bytes it was handed.
    pub fn write_whole_file(
        mut self,
        path: &[u8],
        bytes: &[u8],
        maximum: usize,
    ) -> Result<(Self, WholeFileWrite), ControlError> {
        if bytes.len() > maximum {
            return Ok((self, WholeFileWrite::TooLarge));
        }
        let mut offset: u64 = 0;
        let mut remaining = bytes;
        loop {
            let (chunk, rest) = remaining.split_at(remaining.len().min(MAX_CHUNK_BYTES));
            let last = rest.is_empty();
            let request = FileRequest::Write {
                path: path.into(),
                offset,
                // Only the first record may create the file, and only the last one fixes its
                // length, so a shorter new file cannot keep a tail of the old one.
                create: offset == 0,
                shorten: last,
                bytes: chunk.into(),
            };
            let expected = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
            let (session, outcome) = self.file(request)?;
            self = session;
            match outcome {
                // A short write would leave this loop deriving its next offset from the guest's
                // own accounting; the protocol writes the bytes a record carries or it fails.
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
            offset = offset.saturating_add(expected);
            remaining = rest;
        }
    }
}
