//! CPUID leaf table and MSR table: the CPU configuration restored before registers.

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

pub const MAX_CPUID_ENTRIES: u16 = 256;
pub const MAX_MSR_ENTRIES: u16 = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

impl CpuidEntry {
    pub const ENCODED_LEN: usize = 7 * 4;

    fn write(&self, writer: &mut Writer) {
        for value in [
            self.function,
            self.index,
            self.flags,
            self.eax,
            self.ebx,
            self.ecx,
            self.edx,
        ] {
            writer.put_u32(value);
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let mut values = [0_u32; 7];
        for value in &mut values {
            *value = reader.u32()?;
        }
        let [function, index, flags, eax, ebx, ecx, edx] = values;
        Ok(Self {
            function,
            index,
            flags,
            eax,
            ebx,
            ecx,
            edx,
        })
    }
}

/// Bounded CPUID table with unique `(function, index)` keys.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CpuidEntries {
    entries: Vec<CpuidEntry>,
}

impl CpuidEntries {
    /// # Errors
    ///
    /// Returns [`KvmStateError::TooManyEntries`] or [`KvmStateError::DuplicateEntry`].
    pub fn new(entries: Vec<CpuidEntry>) -> Result<Self, KvmStateError> {
        if entries.len() > usize::from(MAX_CPUID_ENTRIES) {
            return Err(KvmStateError::TooManyEntries {
                field: "cpuid",
                count: entries.len(),
            });
        }
        for (position, entry) in entries.iter().enumerate() {
            let duplicate = entries[..position]
                .iter()
                .any(|e| e.function == entry.function && e.index == entry.index);
            if duplicate {
                return Err(KvmStateError::DuplicateEntry {
                    field: "cpuid",
                    key: u64::from(entry.function) << 32 | u64::from(entry.index),
                });
            }
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[CpuidEntry] {
        &self.entries
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_u16(u16::try_from(self.entries.len()).unwrap_or(MAX_CPUID_ENTRIES));
        for entry in &self.entries {
            entry.write(writer);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let count = reader.count_u16(MAX_CPUID_ENTRIES)?;
        let mut entries = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            entries.push(CpuidEntry::read(reader)?);
        }
        Self::new(entries)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MsrEntry {
    pub index: u32,
    pub value: u64,
}

/// Bounded MSR table with unique indexes in the order KVM must apply them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MsrEntries {
    entries: Vec<MsrEntry>,
}

impl MsrEntries {
    /// # Errors
    ///
    /// Returns [`KvmStateError::TooManyEntries`] or [`KvmStateError::DuplicateEntry`].
    pub fn new(entries: Vec<MsrEntry>) -> Result<Self, KvmStateError> {
        if entries.len() > usize::from(MAX_MSR_ENTRIES) {
            return Err(KvmStateError::TooManyEntries {
                field: "msrs",
                count: entries.len(),
            });
        }
        for (position, entry) in entries.iter().enumerate() {
            if entries[..position].iter().any(|e| e.index == entry.index) {
                return Err(KvmStateError::DuplicateEntry {
                    field: "msrs",
                    key: u64::from(entry.index),
                });
            }
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[MsrEntry] {
        &self.entries
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_u16(u16::try_from(self.entries.len()).unwrap_or(MAX_MSR_ENTRIES));
        for entry in &self.entries {
            writer.put_u32(entry.index);
            writer.put_u64(entry.value);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let count = reader.count_u16(MAX_MSR_ENTRIES)?;
        let mut entries = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            entries.push(MsrEntry {
                index: reader.u32()?,
                value: reader.u64()?,
            });
        }
        Self::new(entries)
    }
}
