//! Fixed-order big-endian wire primitives shared by every snapshot codec.
//!
//! Every decoder treats its input as hostile: each read checks availability first, every
//! length prefix is compared with an explicit bound before any slice or allocation, and
//! presence bytes accept only zero or one.

use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    Truncated { needed: usize, available: usize },
    InvalidPresence(u8),
    LengthExceedsBound { length: u64, bound: u64 },
    TrailingBytes(usize),
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    formatter,
                    "needed {needed} bytes but only {available} remain"
                )
            }
            Self::InvalidPresence(byte) => {
                write!(formatter, "presence byte must be 0 or 1, got {byte}")
            }
            Self::LengthExceedsBound { length, bound } => {
                write!(formatter, "length {length} exceeds bound {bound}")
            }
            Self::TrailingBytes(count) => write!(formatter, "{count} trailing bytes"),
        }
    }
}

impl Error for WireError {}

/// Infallible big-endian encoder.
///
/// Length bounds are enforced by the typed values that own the data, so encoding never
/// fails and never truncates.
#[derive(Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    pub fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn put_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn put_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn put_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn put_i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub fn put_bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn put_presence(&mut self, present: bool) {
        self.bytes.push(u8::from(present));
    }

    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Bounds-checked big-endian decoder over borrowed hostile bytes.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Takes exactly `length` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer than `length` bytes remain.
    pub fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let available = self.remaining();
        if length > available {
            return Err(WireError::Truncated {
                needed: length,
                available,
            });
        }
        let start = self.position;
        // `length <= available` guarantees `start + length <= bytes.len()` without overflow.
        let end = start + length;
        self.position = end;
        Ok(&self.bytes[start..end])
    }

    /// Takes a fixed-width array.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer than `N` bytes remain.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let slice = self.take(N)?;
        let mut array = [0_u8; N];
        array.copy_from_slice(slice);
        Ok(array)
    }

    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when no byte remains.
    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }

    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer than two bytes remain.
    pub fn u16(&mut self) -> Result<u16, WireError> {
        self.array().map(u16::from_be_bytes)
    }

    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer than four bytes remain.
    pub fn u32(&mut self) -> Result<u32, WireError> {
        self.array().map(u32::from_be_bytes)
    }

    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer than eight bytes remain.
    pub fn u64(&mut self) -> Result<u64, WireError> {
        self.array().map(u64::from_be_bytes)
    }

    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer than eight bytes remain.
    pub fn i64(&mut self) -> Result<i64, WireError> {
        self.array().map(i64::from_be_bytes)
    }

    /// Reads one presence byte that must be exactly zero or one.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] or [`WireError::InvalidPresence`].
    pub fn presence(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(WireError::InvalidPresence(other)),
        }
    }

    /// Reads a `u16` count that must not exceed `bound`.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] or [`WireError::LengthExceedsBound`].
    pub fn count_u16(&mut self, bound: u16) -> Result<u16, WireError> {
        let count = self.u16()?;
        bounded(u64::from(count), u64::from(bound)).map(|()| count)
    }

    /// Reads a `u32` length that must not exceed `bound`, then takes that many bytes.
    ///
    /// The bound is checked before availability so an absurd length is reported as a bound
    /// violation rather than being used in any arithmetic or allocation.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::LengthExceedsBound`] or [`WireError::Truncated`].
    pub fn bounded_u32(&mut self, bound: u32) -> Result<&'a [u8], WireError> {
        let length = self.length_u32(bound)?;
        self.take(length)
    }

    /// Reads a `u32` length that must not exceed `bound` without consuming the bytes.
    ///
    /// # Errors
    ///
    /// Returns a short-input error or [`WireError::LengthExceedsBound`].
    pub fn length_u32(&mut self, bound: u32) -> Result<usize, WireError> {
        let length = self.u32()?;
        bounded(u64::from(length), u64::from(bound))?;
        usize::try_from(length).map_err(|_| WireError::LengthExceedsBound {
            length: u64::from(length),
            bound: u64::from(bound),
        })
    }

    /// Rejects any unread trailing byte.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::TrailingBytes`] when input remains.
    pub fn finish(self) -> Result<(), WireError> {
        match self.remaining() {
            0 => Ok(()),
            count => Err(WireError::TrailingBytes(count)),
        }
    }
}

const fn bounded(length: u64, bound: u64) -> Result<(), WireError> {
    if length > bound {
        Err(WireError::LengthExceedsBound { length, bound })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
