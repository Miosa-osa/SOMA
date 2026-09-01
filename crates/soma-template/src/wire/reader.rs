//! The decoding half of the wire primitives: bounds-checked big-endian reads over borrowed
//! hostile bytes.
//!
//! Encoding cannot fail, so it needs no argument. Decoding is where every guarantee is made:
//! each read checks availability before it takes anything, every length prefix is compared with
//! an explicit bound before any slice or allocation, and a presence byte accepts only zero or
//! one. That is the whole subject of this file.

use super::WireError;

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
