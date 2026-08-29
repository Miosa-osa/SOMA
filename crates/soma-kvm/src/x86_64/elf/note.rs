//! `PT_NOTE` walk that finds the Xen `XEN_ELFNOTE_PHYS32_ENTRY` note.
//!
//! Each note is `namesz`, `descsz`, `type`, then the name and descriptor padded to the note
//! alignment. The walk is bounded by the segment's declared file size, which itself must lie
//! inside the image, and every step uses checked arithmetic.

use super::{ElfError, RawProgramHeader, read_u32, read_u64};

/// `XEN_ELFNOTE_PHYS32_ENTRY` from the Xen public ELF-note ABI.
pub(crate) const XEN_ELFNOTE_PHYS32_ENTRY: u32 = 18;
/// The NUL-terminated owner name Xen notes carry.
pub(crate) const XEN_NOTE_NAME: &[u8] = b"Xen\0";
/// Largest `PT_NOTE` segment the walk will inspect.
pub(crate) const MAX_NOTE_SEGMENT_BYTES: usize = 64 * 1024;
const NOTE_HEADER_BYTES: usize = 12;

/// Returns the 32-bit PVH entry if this note segment contains the Xen entry note.
pub(crate) fn pvh_entry(image: &[u8], header: &RawProgramHeader) -> Result<Option<u32>, ElfError> {
    let (offset, size) = header.file_range(image)?;
    if size > MAX_NOTE_SEGMENT_BYTES {
        return Err(ElfError::MalformedNote);
    }
    let align = match header.align {
        0 | 4 => 4,
        8 => 8,
        _ => return Err(ElfError::MalformedNote),
    };
    let segment = &image[offset..offset + size];
    let mut cursor = 0;
    let mut entry = None;
    while cursor < segment.len() {
        let (note, next) = Note::read(segment, cursor, align)?;
        if note.kind == XEN_ELFNOTE_PHYS32_ENTRY && note.name == XEN_NOTE_NAME {
            if entry.is_some() {
                return Err(ElfError::MalformedNote);
            }
            entry = Some(decode_entry(note.descriptor)?);
        }
        cursor = next;
    }
    Ok(entry)
}

struct Note<'a> {
    kind: u32,
    name: &'a [u8],
    descriptor: &'a [u8],
}

impl<'a> Note<'a> {
    fn read(segment: &'a [u8], start: usize, align: usize) -> Result<(Self, usize), ElfError> {
        let header = segment
            .get(
                start
                    ..start
                        .checked_add(NOTE_HEADER_BYTES)
                        .ok_or(ElfError::ArithmeticOverflow)?,
            )
            .ok_or(ElfError::MalformedNote)?;
        let name_size =
            usize::try_from(read_u32(header, 0)).map_err(|_| ElfError::MalformedNote)?;
        let descriptor_size =
            usize::try_from(read_u32(header, 4)).map_err(|_| ElfError::MalformedNote)?;
        let kind = read_u32(header, 8);
        let name_start = start + NOTE_HEADER_BYTES;
        let name_end = name_start
            .checked_add(name_size)
            .ok_or(ElfError::ArithmeticOverflow)?;
        let descriptor_start = align_up(name_end, align)?;
        let descriptor_end = descriptor_start
            .checked_add(descriptor_size)
            .ok_or(ElfError::ArithmeticOverflow)?;
        let next = align_up(descriptor_end, align)?;
        let name = segment
            .get(name_start..name_end)
            .ok_or(ElfError::MalformedNote)?;
        let descriptor = segment
            .get(descriptor_start..descriptor_end)
            .ok_or(ElfError::MalformedNote)?;
        // A trailing note may omit its final padding, so only the content must fit.
        let next = next.min(segment.len());
        Ok((
            Self {
                kind,
                name,
                descriptor,
            },
            next,
        ))
    }
}

/// The Linux note stores a 32-bit entry; a 64-bit descriptor is accepted when it fits.
fn decode_entry(descriptor: &[u8]) -> Result<u32, ElfError> {
    match descriptor.len() {
        4 => Ok(read_u32(descriptor, 0)),
        8 => u32::try_from(read_u64(descriptor, 0)).map_err(|_| ElfError::MalformedNote),
        _ => Err(ElfError::MalformedNote),
    }
}

fn align_up(value: usize, align: usize) -> Result<usize, ElfError> {
    let mask = align - 1;
    value
        .checked_add(mask)
        .map(|padded| padded & !mask)
        .ok_or(ElfError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_descriptor_accepts_four_or_eight_bytes_that_fit() {
        assert_eq!(
            decode_entry(&0x0100_0000_u32.to_le_bytes()),
            Ok(0x0100_0000)
        );
        assert_eq!(
            decode_entry(&0x0100_0000_u64.to_le_bytes()),
            Ok(0x0100_0000)
        );
        assert_eq!(
            decode_entry(&0x1_0000_0000_u64.to_le_bytes()),
            Err(ElfError::MalformedNote)
        );
        assert_eq!(decode_entry(&[1, 2]), Err(ElfError::MalformedNote));
    }

    #[test]
    fn alignment_rounds_up_with_checked_arithmetic() {
        assert_eq!(align_up(13, 4), Ok(16));
        assert_eq!(align_up(16, 4), Ok(16));
        assert_eq!(align_up(usize::MAX, 4), Err(ElfError::ArithmeticOverflow));
    }
}
