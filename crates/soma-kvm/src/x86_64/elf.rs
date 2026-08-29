//! Bounded ELF64 parser for the pinned PVH Linux kernel.
//!
//! The parser owns the fixed subset of ELF the machine contract needs: an `ET_EXEC` `EM_X86_64`
//! little-endian image, at most [`MAX_PROGRAM_HEADERS`] program headers, `PT_LOAD` segments at or
//! above the contract kernel start, and one `XEN_ELFNOTE_PHYS32_ENTRY` note inside a `PT_NOTE`
//! segment. Every offset and length is treated as hostile and checked before it is used.

mod header;
mod note;
#[cfg(test)]
pub(crate) mod synthetic;
#[cfg(test)]
mod tests;

use std::{error::Error, fmt};

use super::layout::KERNEL_START;

pub(crate) const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
pub(crate) const ELFCLASS64: u8 = 2;
pub(crate) const ELFDATA2LSB: u8 = 1;
pub(crate) const ET_EXEC: u16 = 2;
pub(crate) const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const PF_X: u32 = 1;
pub(crate) const ELF_HEADER_BYTES: usize = 64;
pub(crate) const PROGRAM_HEADER_BYTES: usize = 56;
/// A pinned Linux kernel has a handful of program headers; anything larger is rejected.
pub(crate) const MAX_PROGRAM_HEADERS: u16 = 64;

/// Every reason the parser rejects a kernel image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    TooShort,
    BadMagic,
    NotElf64,
    NotLittleEndian,
    NotExecutable,
    NotX86_64,
    BadHeaderSize,
    NoProgramHeaders,
    TooManyProgramHeaders,
    ProgramHeadersOutsideFile,
    ArithmeticOverflow,
    SegmentBelowKernelStart,
    SegmentOutsideFile,
    FileSizeExceedsMemorySize,
    OverlappingSegments,
    NoLoadSegments,
    MalformedNote,
    MissingPvhNote,
    EntryOutsideExecutableSegment,
}

impl fmt::Display for ElfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::TooShort => "image is shorter than an ELF64 header",
            Self::BadMagic => "image does not start with the ELF magic",
            Self::NotElf64 => "image is not ELF64",
            Self::NotLittleEndian => "image is not little-endian",
            Self::NotExecutable => "image is not ET_EXEC",
            Self::NotX86_64 => "image is not EM_X86_64",
            Self::BadHeaderSize => "header sizes do not match ELF64",
            Self::NoProgramHeaders => "image has no program headers",
            Self::TooManyProgramHeaders => "image has more program headers than the bound",
            Self::ProgramHeadersOutsideFile => "program header table lies outside the image",
            Self::ArithmeticOverflow => "a header field overflowed during validation",
            Self::SegmentBelowKernelStart => "a loadable segment lies below the kernel start",
            Self::SegmentOutsideFile => "a loadable segment's file bytes lie outside the image",
            Self::FileSizeExceedsMemorySize => "a segment's file size exceeds its memory size",
            Self::OverlappingSegments => "loadable segments overlap in guest-physical memory",
            Self::NoLoadSegments => "image has no loadable segment",
            Self::MalformedNote => "a PT_NOTE segment is malformed",
            Self::MissingPvhNote => "no XEN_ELFNOTE_PHYS32_ENTRY note is present",
            Self::EntryOutsideExecutableSegment => {
                "the PVH entry lies outside every executable loadable segment"
            }
        };
        formatter.write_str(text)
    }
}

impl Error for ElfError {}

/// One validated `PT_LOAD` segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LoadSegment {
    pub(crate) file_offset: usize,
    pub(crate) file_size: usize,
    pub(crate) guest_address: u64,
    pub(crate) memory_size: u64,
    pub(crate) executable: bool,
}

impl LoadSegment {
    /// Exclusive guest-physical end, already proven not to overflow.
    pub(crate) fn guest_end(self) -> u64 {
        self.guest_address.saturating_add(self.memory_size)
    }
}

/// A validated kernel image: its loadable segments and the 32-bit PVH entry point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PvhKernel {
    segments: Vec<LoadSegment>,
    entry: u32,
}

impl PvhKernel {
    /// Parses and validates `image` without loading anything.
    pub(crate) fn parse(image: &[u8]) -> Result<Self, ElfError> {
        let header = header::Header::parse(image)?;
        let mut segments = Vec::new();
        let mut entry = None;
        for index in 0..header.program_header_count {
            let raw = program_header(image, header.program_header_offset, index)?;
            match raw.kind {
                PT_LOAD => segments.push(raw.into_load_segment(image)?),
                PT_NOTE => {
                    if let Some(found) = note::pvh_entry(image, &raw)? {
                        entry = Some(found);
                    }
                }
                _ => {}
            }
        }
        if segments.is_empty() {
            return Err(ElfError::NoLoadSegments);
        }
        reject_overlap(&mut segments)?;
        let entry = entry.ok_or(ElfError::MissingPvhNote)?;
        if !segments
            .iter()
            .any(|segment| segment.executable && segment.contains(u64::from(entry)))
        {
            return Err(ElfError::EntryOutsideExecutableSegment);
        }
        Ok(Self { segments, entry })
    }

    pub(crate) fn segments(&self) -> &[LoadSegment] {
        &self.segments
    }

    pub(crate) const fn entry(&self) -> u32 {
        self.entry
    }
}

impl LoadSegment {
    fn contains(self, address: u64) -> bool {
        address >= self.guest_address && address < self.guest_end()
    }
}

/// One raw program header with fields decoded but not yet validated.
pub(crate) struct RawProgramHeader {
    kind: u32,
    flags: u32,
    pub(crate) offset: u64,
    pub(crate) physical_address: u64,
    pub(crate) file_size: u64,
    memory_size: u64,
    pub(crate) align: u64,
}

fn program_header(image: &[u8], table: usize, index: u16) -> Result<RawProgramHeader, ElfError> {
    let start = table
        .checked_add(usize::from(index).saturating_mul(PROGRAM_HEADER_BYTES))
        .ok_or(ElfError::ArithmeticOverflow)?;
    let end = start
        .checked_add(PROGRAM_HEADER_BYTES)
        .ok_or(ElfError::ArithmeticOverflow)?;
    let raw = image
        .get(start..end)
        .ok_or(ElfError::ProgramHeadersOutsideFile)?;
    Ok(RawProgramHeader {
        kind: read_u32(raw, 0),
        flags: read_u32(raw, 4),
        offset: read_u64(raw, 8),
        physical_address: read_u64(raw, 24),
        file_size: read_u64(raw, 32),
        memory_size: read_u64(raw, 40),
        align: read_u64(raw, 48),
    })
}

impl RawProgramHeader {
    fn into_load_segment(self, image: &[u8]) -> Result<LoadSegment, ElfError> {
        if self.physical_address < KERNEL_START {
            return Err(ElfError::SegmentBelowKernelStart);
        }
        if self.file_size > self.memory_size {
            return Err(ElfError::FileSizeExceedsMemorySize);
        }
        self.physical_address
            .checked_add(self.memory_size)
            .filter(|end| u32::try_from(*end).is_ok())
            .ok_or(ElfError::ArithmeticOverflow)?;
        let (file_offset, file_size) = self.file_range(image)?;
        Ok(LoadSegment {
            file_offset,
            file_size,
            guest_address: self.physical_address,
            memory_size: self.memory_size,
            executable: self.flags & PF_X != 0,
        })
    }

    /// Checks that the segment's file bytes lie inside `image` and returns them as `usize`.
    pub(crate) fn file_range(&self, image: &[u8]) -> Result<(usize, usize), ElfError> {
        let offset = usize::try_from(self.offset).map_err(|_| ElfError::SegmentOutsideFile)?;
        let size = usize::try_from(self.file_size).map_err(|_| ElfError::SegmentOutsideFile)?;
        let end = offset
            .checked_add(size)
            .ok_or(ElfError::ArithmeticOverflow)?;
        if end > image.len() {
            return Err(ElfError::SegmentOutsideFile);
        }
        Ok((offset, size))
    }
}

/// Sorts segments by guest address and rejects any pair whose non-empty ranges intersect.
fn reject_overlap(segments: &mut [LoadSegment]) -> Result<(), ElfError> {
    segments.sort_by_key(|segment| segment.guest_address);
    for pair in segments.windows(2) {
        let (lower, upper) = (pair[0], pair[1]);
        if lower.memory_size != 0
            && upper.memory_size != 0
            && lower.guest_end() > upper.guest_address
        {
            return Err(ElfError::OverlappingSegments);
        }
    }
    Ok(())
}

pub(crate) fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut raw = [0_u8; 4];
    raw.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(raw)
}

pub(crate) fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(raw)
}
