//! ELF64 file-header validation.

use super::{
    ELF_HEADER_BYTES, ELF_MAGIC, ELFCLASS64, ELFDATA2LSB, EM_X86_64, ET_EXEC, ElfError,
    MAX_PROGRAM_HEADERS, PROGRAM_HEADER_BYTES, read_u16, read_u64,
};

pub(crate) struct Header {
    pub(crate) program_header_offset: usize,
    pub(crate) program_header_count: u16,
}

impl Header {
    pub(crate) fn parse(image: &[u8]) -> Result<Self, ElfError> {
        let header = image.get(..ELF_HEADER_BYTES).ok_or(ElfError::TooShort)?;
        if header[..4] != ELF_MAGIC {
            return Err(ElfError::BadMagic);
        }
        if header[4] != ELFCLASS64 {
            return Err(ElfError::NotElf64);
        }
        if header[5] != ELFDATA2LSB {
            return Err(ElfError::NotLittleEndian);
        }
        if read_u16(header, 16) != ET_EXEC {
            return Err(ElfError::NotExecutable);
        }
        if read_u16(header, 18) != EM_X86_64 {
            return Err(ElfError::NotX86_64);
        }
        let header_size = usize::from(read_u16(header, 52));
        let entry_size = usize::from(read_u16(header, 54));
        if header_size != ELF_HEADER_BYTES || entry_size != PROGRAM_HEADER_BYTES {
            return Err(ElfError::BadHeaderSize);
        }
        let count = read_u16(header, 56);
        if count == 0 {
            return Err(ElfError::NoProgramHeaders);
        }
        if count > MAX_PROGRAM_HEADERS {
            return Err(ElfError::TooManyProgramHeaders);
        }
        let offset = usize::try_from(read_u64(header, 32))
            .map_err(|_| ElfError::ProgramHeadersOutsideFile)?;
        let table_bytes = usize::from(count)
            .checked_mul(PROGRAM_HEADER_BYTES)
            .ok_or(ElfError::ArithmeticOverflow)?;
        let table_end = offset
            .checked_add(table_bytes)
            .ok_or(ElfError::ArithmeticOverflow)?;
        if offset < ELF_HEADER_BYTES || table_end > image.len() {
            return Err(ElfError::ProgramHeadersOutsideFile);
        }
        Ok(Self {
            program_header_offset: offset,
            program_header_count: count,
        })
    }
}
