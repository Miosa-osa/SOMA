//! Read one bounded range of a guest file over the repaired control session.

use super::{
    ControlError, ControlFailureClass, ControlStage, FileOutcome, FileRequest, HostControlIo,
    MAX_CHUNK_BYTES, RangeRead, RepairedHostControl,
};

impl<I: HostControlIo> RepairedHostControl<I> {
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
}
