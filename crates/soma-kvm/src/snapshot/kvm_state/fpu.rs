//! Legacy FPU state, extended control registers, and the bounded XSAVE area.

use super::{KvmStateError, invalid};
use crate::snapshot::wire::{Reader, Writer};

pub const MAX_XCRS: usize = 16;
/// The base `kvm_xsave` region is 4096 bytes.
pub const MIN_XSAVE_BYTES: u32 = 4096;
pub const MAX_XSAVE_BYTES: u32 = 64 * 1024;

/// Legacy x87 and SSE state as reported by `KVM_GET_FPU`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Fpu {
    pub fpr: [[u8; 16]; 8],
    pub fcw: u16,
    pub fsw: u16,
    pub ftwx: u8,
    pub last_opcode: u16,
    pub last_ip: u64,
    pub last_dp: u64,
    pub xmm: [[u8; 16]; 16],
    pub mxcsr: u32,
}

impl Fpu {
    pub const ENCODED_LEN: usize = 128 + 2 + 2 + 1 + 2 + 8 + 8 + 256 + 4;

    pub(crate) fn write(&self, writer: &mut Writer) {
        for register in &self.fpr {
            writer.put_bytes(register);
        }
        writer.put_u16(self.fcw);
        writer.put_u16(self.fsw);
        writer.put_u8(self.ftwx);
        writer.put_u16(self.last_opcode);
        writer.put_u64(self.last_ip);
        writer.put_u64(self.last_dp);
        for register in &self.xmm {
            writer.put_bytes(register);
        }
        writer.put_u32(self.mxcsr);
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let mut fpr = [[0_u8; 16]; 8];
        for register in &mut fpr {
            *register = reader.array()?;
        }
        let mut fpu = Self {
            fpr,
            fcw: reader.u16()?,
            fsw: reader.u16()?,
            ftwx: reader.u8()?,
            last_opcode: reader.u16()?,
            last_ip: reader.u64()?,
            last_dp: reader.u64()?,
            ..Self::default()
        };
        for register in &mut fpu.xmm {
            *register = reader.array()?;
        }
        fpu.mxcsr = reader.u32()?;
        Ok(fpu)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XcrEntry {
    pub index: u32,
    pub value: u64,
}

/// Bounded list of extended control registers plus the KVM flags word.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Xcrs {
    flags: u32,
    entries: Vec<XcrEntry>,
}

impl Xcrs {
    /// # Errors
    ///
    /// Returns [`KvmStateError::TooManyEntries`] or [`KvmStateError::DuplicateEntry`].
    pub fn new(flags: u32, entries: Vec<XcrEntry>) -> Result<Self, KvmStateError> {
        if entries.len() > MAX_XCRS {
            return Err(KvmStateError::TooManyEntries {
                field: "xcrs",
                count: entries.len(),
            });
        }
        for (position, entry) in entries.iter().enumerate() {
            if entries[..position].iter().any(|e| e.index == entry.index) {
                return Err(KvmStateError::DuplicateEntry {
                    field: "xcrs",
                    key: u64::from(entry.index),
                });
            }
        }
        Ok(Self { flags, entries })
    }

    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    #[must_use]
    pub fn entries(&self) -> &[XcrEntry] {
        &self.entries
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_u32(self.flags);
        writer.put_u8(u8::try_from(self.entries.len()).unwrap_or(u8::MAX));
        for entry in &self.entries {
            writer.put_u32(entry.index);
            writer.put_u64(entry.value);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let flags = reader.u32()?;
        let count = reader.u8()?;
        if usize::from(count) > MAX_XCRS {
            return Err(KvmStateError::TooManyEntries {
                field: "xcrs",
                count: usize::from(count),
            });
        }
        let mut entries = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            entries.push(XcrEntry {
                index: reader.u32()?,
                value: reader.u64()?,
            });
        }
        Self::new(flags, entries)
    }
}

/// Bounded opaque XSAVE area; its exact byte length is recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XsaveArea {
    bytes: Vec<u8>,
}

impl XsaveArea {
    /// Accepts a length between [`MIN_XSAVE_BYTES`] and [`MAX_XSAVE_BYTES`], a multiple of 4.
    ///
    /// # Errors
    ///
    /// Returns [`KvmStateError::InvalidField`] for any other length.
    pub fn new(bytes: Vec<u8>) -> Result<Self, KvmStateError> {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if length < u64::from(MIN_XSAVE_BYTES)
            || length > u64::from(MAX_XSAVE_BYTES)
            || length % 4 != 0
        {
            return Err(invalid("xsave.len", length));
        }
        Ok(Self { bytes })
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        // `new` bounded the length to `MAX_XSAVE_BYTES`, which fits in `u32`.
        writer.put_u32(u32::try_from(self.bytes.len()).unwrap_or(MAX_XSAVE_BYTES));
        writer.put_bytes(&self.bytes);
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let bytes = reader.bounded_u32(MAX_XSAVE_BYTES)?;
        Self::new(bytes.to_vec())
    }
}
