use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

/// The ELF and PVH contract version enforced by this verifier.
pub const ELF_PVH_CONTRACT_VERSION: u16 = 1;
/// Every loadable segment must start at or above this guest-physical address.
pub const MINIMUM_LOAD_PADDR: u64 = 0x0100_0000;

const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3e;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const PF_X: u32 = 1;
const XEN_ELFNOTE_PHYS32_ENTRY: u32 = 18;
const MAX_PROGRAM_HEADERS: u16 = 64;
const MAX_NOTE_BYTES: u64 = 64 * 1024;

/// One verified loadable ELF segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    /// The guest-physical load address.
    pub paddr: u64,
    /// The bytes present in the file.
    pub file_size: u64,
    /// The bytes occupied in memory, including zero fill.
    pub memory_size: u64,
    /// Whether the segment is executable.
    pub executable: bool,
}

/// One kernel image that satisfied the `x86_64` PVH contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedKernel {
    /// The 32-bit PVH entry point carried by `XEN_ELFNOTE_PHYS32_ENTRY`.
    pub pvh_entry: u32,
    /// The loadable segments in program-header order.
    pub segments: Vec<LoadSegment>,
    /// The digest of the exact kernel bytes.
    pub digest: Sha256Digest,
    /// The exact kernel byte length.
    pub size: u64,
}

/// Verifies an uncompressed `x86_64` ELF kernel against machine contract v1.
///
/// # Errors
///
/// Returns [`CompileErrorKind::InvalidInput`] for a malformed or truncated ELF,
/// [`CompileErrorKind::Unsupported`] for the wrong class, type, or machine, and
/// [`CompileErrorKind::Integrity`] when segments or the PVH note violate the contract.
pub fn verify_kernel(bytes: &[u8]) -> Result<VerifiedKernel, CompileError> {
    let header = Elf::new(bytes)?;
    let mut segments = Vec::new();
    let mut entry = None;
    for index in 0..header.program_header_count {
        let program = header.program_header(index)?;
        match program.kind {
            PT_LOAD => segments.push(header.load_segment(&program)?),
            PT_NOTE => {
                if let Some(value) = header.pvh_note(&program)?
                    && entry.replace(value).is_some()
                {
                    return Err(integrity());
                }
            }
            _ => {}
        }
    }
    let pvh_entry = entry.ok_or_else(integrity)?;
    if segments.is_empty() {
        return Err(integrity());
    }
    require_non_overlapping(&segments)?;
    let inside_executable = segments.iter().any(|segment| {
        segment.executable
            && u64::from(pvh_entry) >= segment.paddr
            && u64::from(pvh_entry) < segment.paddr + segment.memory_size
    });
    if !inside_executable {
        return Err(integrity());
    }
    Ok(VerifiedKernel {
        pvh_entry,
        segments,
        digest: Sha256Digest::of(bytes),
        size: u64::try_from(bytes.len()).map_err(|_| invalid())?,
    })
}

fn require_non_overlapping(segments: &[LoadSegment]) -> Result<(), CompileError> {
    let mut ranges: Vec<(u64, u64)> = segments
        .iter()
        .map(|segment| {
            segment
                .paddr
                .checked_add(segment.memory_size)
                .map(|end| (segment.paddr, end))
                .ok_or_else(integrity)
        })
        .collect::<Result<_, _>>()?;
    ranges.sort_unstable();
    for pair in ranges.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(integrity());
        }
    }
    Ok(())
}

struct ProgramHeader {
    kind: u32,
    flags: u32,
    offset: u64,
    paddr: u64,
    file_size: u64,
    memory_size: u64,
}

struct Elf<'a> {
    bytes: &'a [u8],
    program_header_offset: u64,
    program_header_size: u16,
    program_header_count: u16,
}

impl<'a> Elf<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, CompileError> {
        if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" {
            return Err(invalid());
        }
        if bytes[4] != 2 || bytes[5] != 1 || bytes[6] != 1 {
            return Err(unsupported());
        }
        let kind = u16::from_le_bytes([bytes[16], bytes[17]]);
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        if kind != ET_EXEC || machine != EM_X86_64 {
            return Err(unsupported());
        }
        let version = u32::from_le_bytes(bytes[20..24].try_into().map_err(|_| invalid())?);
        if version != 1 {
            return Err(invalid());
        }
        let program_header_offset = read_u64(bytes, 32)?;
        let program_header_size = u16::from_le_bytes([bytes[54], bytes[55]]);
        let program_header_count = u16::from_le_bytes([bytes[56], bytes[57]]);
        if program_header_size != 56 || program_header_count > MAX_PROGRAM_HEADERS {
            return Err(unsupported());
        }
        if program_header_count == 0 {
            return Err(integrity());
        }
        Ok(Self {
            bytes,
            program_header_offset,
            program_header_size,
            program_header_count,
        })
    }

    fn program_header(&self, index: u16) -> Result<ProgramHeader, CompileError> {
        let offset = self
            .program_header_offset
            .checked_add(u64::from(index) * u64::from(self.program_header_size))
            .ok_or_else(invalid)?;
        let start = usize::try_from(offset).map_err(|_| invalid())?;
        let header = self
            .bytes
            .get(start..start.checked_add(56).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        Ok(ProgramHeader {
            kind: read_u32(header, 0)?,
            flags: read_u32(header, 4)?,
            offset: read_u64(header, 8)?,
            paddr: read_u64(header, 24)?,
            file_size: read_u64(header, 32)?,
            memory_size: read_u64(header, 40)?,
        })
    }

    fn segment_bytes(&self, program: &ProgramHeader) -> Result<&'a [u8], CompileError> {
        let start = usize::try_from(program.offset).map_err(|_| invalid())?;
        let length = usize::try_from(program.file_size).map_err(|_| invalid())?;
        self.bytes
            .get(start..start.checked_add(length).ok_or_else(invalid)?)
            .ok_or_else(invalid)
    }

    fn load_segment(&self, program: &ProgramHeader) -> Result<LoadSegment, CompileError> {
        self.segment_bytes(program)?;
        if program.memory_size < program.file_size || program.paddr < MINIMUM_LOAD_PADDR {
            return Err(integrity());
        }
        program
            .paddr
            .checked_add(program.memory_size)
            .ok_or_else(integrity)?;
        Ok(LoadSegment {
            paddr: program.paddr,
            file_size: program.file_size,
            memory_size: program.memory_size,
            executable: program.flags & PF_X != 0,
        })
    }

    fn pvh_note(&self, program: &ProgramHeader) -> Result<Option<u32>, CompileError> {
        if program.file_size > MAX_NOTE_BYTES {
            return Err(unsupported());
        }
        let mut notes = self.segment_bytes(program)?;
        let mut found = None;
        while notes.len() >= 12 {
            let name_size = usize::try_from(read_u32(notes, 0)?).map_err(|_| invalid())?;
            let desc_size = usize::try_from(read_u32(notes, 4)?).map_err(|_| invalid())?;
            let kind = read_u32(notes, 8)?;
            let name_end = 12_usize.checked_add(name_size).ok_or_else(invalid)?;
            let desc_start = align4(name_end)?;
            let desc_end = desc_start.checked_add(desc_size).ok_or_else(invalid)?;
            let name = notes.get(12..name_end).ok_or_else(invalid)?;
            let desc = notes.get(desc_start..desc_end).ok_or_else(invalid)?;
            if name == b"Xen\0" && kind == XEN_ELFNOTE_PHYS32_ENTRY {
                let value = match desc.len() {
                    4 => u64::from(read_u32(desc, 0)?),
                    8 => read_u64(desc, 0)?,
                    _ => return Err(integrity()),
                };
                let value = u32::try_from(value).map_err(|_| integrity())?;
                if found.replace(value).is_some() {
                    return Err(integrity());
                }
            }
            notes = notes
                .get(align4(desc_end)?.min(notes.len())..)
                .unwrap_or(&[]);
        }
        Ok(found)
    }
}

fn align4(value: usize) -> Result<usize, CompileError> {
    value.checked_add(3).map(|sum| sum & !3).ok_or_else(invalid)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CompileError> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(invalid)?;
    Ok(u32::from_le_bytes(slice.try_into().map_err(|_| invalid())?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, CompileError> {
    let slice = bytes.get(offset..offset + 8).ok_or_else(invalid)?;
    Ok(u64::from_le_bytes(slice.try_into().map_err(|_| invalid())?))
}

const fn invalid() -> CompileError {
    CompileError::new(CompilePhase::VerifyKernel, CompileErrorKind::InvalidInput)
}

const fn unsupported() -> CompileError {
    CompileError::new(CompilePhase::VerifyKernel, CompileErrorKind::Unsupported)
}

const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::VerifyKernel, CompileErrorKind::Integrity)
}
