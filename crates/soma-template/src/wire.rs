//! Fixed-order big-endian wire primitives shared by the lock, module, and content digests.
//!
//! Every decoder treats its input as hostile: each read checks availability first, every
//! length prefix is compared with an explicit bound before any slice or allocation, and
//! presence bytes accept only zero or one.

pub use reader::Reader;

// The decoder is beside this file. The encoder below cannot fail; the decoder is the half that
// reads hostile bytes and must refuse them, and that asymmetry is the whole reason this module
// exists, so the two are not read as one.
mod reader;

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
