//! Fixed-order big-endian wire primitives shared by the lock, module, and content digests.
//!
//! Every decoder treats its input as hostile: each read checks availability first, every
//! length prefix is compared with an explicit bound before any slice or allocation, and
//! presence bytes accept only zero or one.

use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    ShortInput { needed: usize, available: usize },
    InvalidPresence(u8),
    LengthExceedsBound { length: u64, bound: u64 },
    InvalidUtf8,
    TrailingBytes(usize),
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortInput { needed, available } => {
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
            Self::InvalidUtf8 => formatter.write_str("string is not valid UTF-8"),
            Self::TrailingBytes(count) => write!(formatter, "{count} trailing bytes"),
        }
    }
}

impl Error for WireError {}

/// Infallible big-endian encoder.
///
/// Length bounds are enforced by the typed values that own the data, so encoding never
/// fails and never shortens a value.
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

    pub fn put_bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn put_presence(&mut self, present: bool) {
        self.bytes.push(u8::from(present));
    }

    /// Writes a `u32` length prefix followed by the bytes.
    ///
    /// Typed owners bound every string far below `u32::MAX`, so the conversion cannot fail
    /// for values produced by this crate; an impossible overflow saturates rather than panics.
    pub fn put_string(&mut self, value: &str) {
        let length = u32::try_from(value.len()).unwrap_or(u32::MAX);
        self.put_u32(length);
        self.put_bytes(value.as_bytes());
    }

    pub fn put_optional_string(&mut self, value: Option<&str>) {
        self.put_presence(value.is_some());
        if let Some(value) = value {
            self.put_string(value);
        }
    }

    /// Writes a `u16` element count; typed owners bound every list below `u16::MAX`.
    pub fn put_count(&mut self, count: usize) {
        self.put_u16(u16::try_from(count).unwrap_or(u16::MAX));
    }

    pub fn put_strings(&mut self, values: &[String]) {
        self.put_count(values.len());
        for value in values {
            self.put_string(value);
        }
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
    /// Returns [`WireError::ShortInput`] when fewer than `length` bytes remain.
    pub fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let available = self.remaining();
        if length > available {
            return Err(WireError::ShortInput {
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
    /// Returns [`WireError::ShortInput`] when fewer than `N` bytes remain.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let slice = self.take(N)?;
        let mut array = [0_u8; N];
        array.copy_from_slice(slice);
        Ok(array)
    }

    /// # Errors
    ///
    /// Returns [`WireError::ShortInput`] when no byte remains.
    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }

    /// # Errors
    ///
    /// Returns [`WireError::ShortInput`] when fewer than two bytes remain.
    pub fn u16(&mut self) -> Result<u16, WireError> {
        self.array().map(u16::from_be_bytes)
    }

    /// # Errors
    ///
    /// Returns [`WireError::ShortInput`] when fewer than four bytes remain.
    pub fn u32(&mut self) -> Result<u32, WireError> {
        self.array().map(u32::from_be_bytes)
    }

    /// # Errors
    ///
    /// Returns [`WireError::ShortInput`] when fewer than eight bytes remain.
    pub fn u64(&mut self) -> Result<u64, WireError> {
        self.array().map(u64::from_be_bytes)
    }

    /// Reads one presence byte that must be exactly zero or one.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::ShortInput`] or [`WireError::InvalidPresence`].
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
    /// Returns [`WireError::ShortInput`] or [`WireError::LengthExceedsBound`].
    pub fn count(&mut self, bound: usize) -> Result<usize, WireError> {
        let count = usize::from(self.u16()?);
        if count > bound {
            return Err(WireError::LengthExceedsBound {
                length: count as u64,
                bound: bound as u64,
            });
        }
        Ok(count)
    }

    /// Reads a `u32` length that must not exceed `bound`, then takes that many bytes.
    ///
    /// The bound is checked before availability so an absurd length is reported as a bound
    /// violation rather than being used in any arithmetic or allocation.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::LengthExceedsBound`] or [`WireError::ShortInput`].
    pub fn bounded(&mut self, bound: usize) -> Result<&'a [u8], WireError> {
        let length = self.u32()?;
        let exceeds = WireError::LengthExceedsBound {
            length: u64::from(length),
            bound: bound as u64,
        };
        if u64::from(length) > bound as u64 {
            return Err(exceeds);
        }
        let length = usize::try_from(length).map_err(|_| exceeds)?;
        self.take(length)
    }

    /// Reads a bounded UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns a bound, short-input, or [`WireError::InvalidUtf8`] failure.
    pub fn string(&mut self, bound: usize) -> Result<String, WireError> {
        let bytes = self.bounded(bound)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| WireError::InvalidUtf8)
    }

    /// Reads a presence byte followed by an optional bounded string.
    ///
    /// # Errors
    ///
    /// Returns any failure of [`Reader::presence`] or [`Reader::string`].
    pub fn optional_string(&mut self, bound: usize) -> Result<Option<String>, WireError> {
        if self.presence()? {
            self.string(bound).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Reads a bounded count followed by that many bounded strings.
    ///
    /// # Errors
    ///
    /// Returns any failure of [`Reader::count`] or [`Reader::string`].
    pub fn strings(
        &mut self,
        count_bound: usize,
        string_bound: usize,
    ) -> Result<Vec<String>, WireError> {
        let count = self.count(count_bound)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.string(string_bound)?);
        }
        Ok(values)
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
