//! Parser tests over synthetic ELF images.

use super::{
    ELF_HEADER_BYTES, ElfError, MAX_PROGRAM_HEADERS, PROGRAM_HEADER_BYTES, PvhKernel,
    note::{XEN_ELFNOTE_PHYS32_ENTRY, XEN_NOTE_NAME},
    synthetic::{PF_R, Segment, SyntheticElf},
};
use crate::x86_64::layout::KERNEL_START;

#[test]
fn parses_a_kernel_like_image() {
    let entry = u32::try_from(KERNEL_START).unwrap() + 16;
    let kernel = PvhKernel::parse(&SyntheticElf::kernel(entry).build()).unwrap();
    assert_eq!(kernel.entry(), entry);
    assert_eq!(kernel.segments().len(), 1);
    let segment = kernel.segments()[0];
    assert_eq!(segment.guest_address, KERNEL_START);
    assert_eq!(segment.file_size, 64);
    assert_eq!(segment.memory_size, 64 + 4096);
    assert!(segment.executable);
}

#[test]
fn accepts_an_eight_byte_note_descriptor_and_eight_byte_alignment() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry);
    elf.note = Some((
        XEN_NOTE_NAME.to_vec(),
        XEN_ELFNOTE_PHYS32_ENTRY,
        u64::from(entry).to_le_bytes().to_vec(),
    ));
    assert_eq!(PvhKernel::parse(&elf.build()).unwrap().entry(), entry);
    elf.note_align = 8;
    assert_eq!(PvhKernel::parse(&elf.build()).unwrap().entry(), entry);
    elf.note_align = 16;
    assert_eq!(PvhKernel::parse(&elf.build()), Err(ElfError::MalformedNote));
}

#[test]
fn rejects_missing_note_wrong_name_and_wrong_type() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry);
    elf.note = None;
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::MissingPvhNote)
    );
    elf.note = Some((
        b"Linux\0".to_vec(),
        XEN_ELFNOTE_PHYS32_ENTRY,
        entry.to_le_bytes().to_vec(),
    ));
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::MissingPvhNote)
    );
    elf.note = Some((XEN_NOTE_NAME.to_vec(), 17, entry.to_le_bytes().to_vec()));
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::MissingPvhNote)
    );
    elf.note = Some((XEN_NOTE_NAME.to_vec(), XEN_ELFNOTE_PHYS32_ENTRY, vec![1, 2]));
    assert_eq!(PvhKernel::parse(&elf.build()), Err(ElfError::MalformedNote));
}

#[test]
fn rejects_entry_outside_an_executable_segment() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry + 64 + 4096);
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::EntryOutsideExecutableSegment)
    );
    elf.note = Some((
        XEN_NOTE_NAME.to_vec(),
        XEN_ELFNOTE_PHYS32_ENTRY,
        entry.to_le_bytes().to_vec(),
    ));
    elf.segments[0].flags = PF_R;
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::EntryOutsideExecutableSegment)
    );
}

#[test]
fn rejects_bad_segment_geometry() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry);
    elf.segments[0].address = KERNEL_START - 4096;
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::SegmentBelowKernelStart)
    );
    let mut elf = SyntheticElf::kernel(entry);
    elf.segments.push(Segment {
        address: KERNEL_START + 32,
        data: vec![0; 16],
        extra_memory: 0,
        flags: PF_R,
    });
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::OverlappingSegments)
    );
    let mut elf = SyntheticElf::kernel(entry);
    elf.segments[0].address = u64::from(u32::MAX) - 8;
    assert_eq!(
        PvhKernel::parse(&elf.build()),
        Err(ElfError::ArithmeticOverflow)
    );
    let mut image = SyntheticElf::kernel(entry).build();
    // Shrink the memory size of the load segment below its file size.
    let load_header = ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES;
    image[load_header + 40..load_header + 48].copy_from_slice(&8_u64.to_le_bytes());
    assert_eq!(
        PvhKernel::parse(&image),
        Err(ElfError::FileSizeExceedsMemorySize)
    );
}

#[test]
fn rejects_wrong_header_identity_and_bounds() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry);
    elf.machine = 183;
    assert_eq!(PvhKernel::parse(&elf.build()), Err(ElfError::NotX86_64));
    elf.machine = super::EM_X86_64;
    elf.kind = 3;
    assert_eq!(PvhKernel::parse(&elf.build()), Err(ElfError::NotExecutable));
    let mut image = SyntheticElf::kernel(entry).build();
    image[4] = 1;
    assert_eq!(PvhKernel::parse(&image), Err(ElfError::NotElf64));
    image[4] = 2;
    image[5] = 2;
    assert_eq!(PvhKernel::parse(&image), Err(ElfError::NotLittleEndian));
    image[5] = 1;
    image[0] = 0;
    assert_eq!(PvhKernel::parse(&image), Err(ElfError::BadMagic));
    image[0] = 0x7f;
    image[56..58].copy_from_slice(&(MAX_PROGRAM_HEADERS + 1).to_le_bytes());
    assert_eq!(
        PvhKernel::parse(&image),
        Err(ElfError::TooManyProgramHeaders)
    );
    image[56..58].copy_from_slice(&0_u16.to_le_bytes());
    assert_eq!(PvhKernel::parse(&image), Err(ElfError::NoProgramHeaders));
    image[56..58].copy_from_slice(&64_u16.to_le_bytes());
    assert_eq!(
        PvhKernel::parse(&image),
        Err(ElfError::ProgramHeadersOutsideFile)
    );
    assert_eq!(PvhKernel::parse(&[]), Err(ElfError::TooShort));
}

#[test]
fn truncation_sweep_never_panics_and_never_accepts_a_partial_image() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let image = SyntheticElf::kernel(entry).build();
    for length in 0..image.len() {
        assert!(
            PvhKernel::parse(&image[..length]).is_err(),
            "length {length}"
        );
    }
    assert!(PvhKernel::parse(&image).is_ok());
}

#[test]
fn bit_flip_sweep_never_panics() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let image = SyntheticElf::kernel(entry).build();
    for index in 0..image.len() {
        for bit in 0..8 {
            let mut mutated = image.clone();
            mutated[index] ^= 1 << bit;
            let _ = PvhKernel::parse(&mutated);
        }
    }
}
