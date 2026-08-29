use core::fmt;

use crate::Error;

/// Maximum bytes carried by one authenticated stdout or stderr message.
pub const MAX_OUTPUT_CHUNK_BYTES: usize = 4096;

/// One nonempty bounded binary output chunk.
#[derive(Clone, Eq, PartialEq)]
pub struct OutputChunk(Box<[u8]>);

impl OutputChunk {
    /// Creates one bounded binary output chunk.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidOutputChunk`] for empty or oversized output.
    pub fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_OUTPUT_CHUNK_BYTES {
            return Err(Error::InvalidOutputChunk);
        }
        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Returns the exact binary output bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Self::new(bytes.to_vec()).map_err(|_| Error::ApplicationMessageRejected)
    }
}

impl fmt::Debug for OutputChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputChunk")
            .field("bytes", &self.0.len())
            .finish()
    }
}
