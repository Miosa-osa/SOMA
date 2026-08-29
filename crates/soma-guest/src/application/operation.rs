use core::fmt;

use crate::Error;

/// A nonzero canonical identity for one launch or Execute operation.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct OperationId([u8; 16]);

impl OperationId {
    /// Validates raw operation identity bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidOperation`] for the reserved all-zero identity.
    pub fn new(bytes: [u8; 16]) -> Result<Self, Error> {
        if bytes == [0; 16] {
            return Err(Error::InvalidOperation);
        }
        Ok(Self(bytes))
    }

    /// Returns the canonical non-secret identity bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperationId(..)")
    }
}
