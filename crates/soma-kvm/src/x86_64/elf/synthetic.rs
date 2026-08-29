//! Synthetic ELF64 image builder for parser and loader tests.
#![allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]

use super::{
    ELF_HEADER_BYTES, PROGRAM_HEADER_BYTES, PT_LOAD, PT_NOTE,
    note::{XEN_ELFNOTE_PHYS32_ENTRY, XEN_NOTE_NAME},
};
use crate::x86_64::layout::KERNEL_START;

pub(crate) const PF_R: u32 = 4;
pub(crate) const PF_X: u32 = 1;

/// One synthetic loadable segment: guest address, file bytes, extra zero-filled bytes, flags.
#[derive(Clone)]
pub(crate) struct Segment {
    pub(crate) address: u64,
    pub(crate) data: Vec<u8>,
    pub(crate) extra_memory: u64,
    pub(crate) flags: u32,
}

/// A synthetic ELF64 `ET_EXEC` image builder.
pub(crate) struct SyntheticElf {
    pub(crate) segments: Vec<Segment>,
    pub(crate) note: Option<(Vec<u8>, u32, Vec<u8>)>,
    pub(crate) machine: u16,
    pub(crate) kind: u16,
    pub(crate) note_align: u64,
}

impl SyntheticElf {
    /// A kernel-like image: one executable segment at the contract kernel start and a PVH note.
    pub(crate) fn kernel(entry: u32) -> Self {
        Self {
            segments: vec![Segment {
                address: KERNEL_START,
                data: vec![0xf4; 64],
                extra_memory: 4096,
                flags: PF_R | PF_X,
            }],
            note: Some((
                XEN_NOTE_NAME.to_vec(),
                XEN_ELFNOTE_PHYS32_ENTRY,
                entry.to_le_bytes().to_vec(),
            )),
            machine: super::EM_X86_64,
            kind: super::ET_EXEC,
            note_align: 4,
        }
    }

    pub(crate) fn note_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        if let Some((name, kind, descriptor)) = &self.note {
            bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(descriptor.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(name);
            bytes.resize(bytes.len().next_multiple_of(4), 0);
            bytes.extend_from_slice(descriptor);
            bytes.resize(bytes.len().next_multiple_of(4), 0);
        }
        bytes
    }

    pub(crate) fn build(&self) -> Vec<u8> {
        let note = self.note_bytes();
        let header_count = self.segments.len() + usize::from(!note.is_empty());
        let mut headers = Vec::new();
        let mut body = Vec::new();
        let data_start = ELF_HEADER_BYTES + header_count * PROGRAM_HEADER_BYTES;
        if !note.is_empty() {
            let offset = data_start + body.len();
            headers.extend(program_header(
                PT_NOTE,
                PF_R,
                offset,
                0,
                note.len(),
                0,
                self.note_align,
            ));
            body.extend_from_slice(&note);
        }
        for segment in &self.segments {
            let offset = data_start + body.len();
            headers.extend(program_header(
                PT_LOAD,
                segment.flags,
                offset,
                segment.address,
                segment.data.len(),
                segment.extra_memory,
                4096,
            ));
            body.extend_from_slice(&segment.data);
        }
        let mut image = elf_header(self.kind, self.machine, header_count);
        image.extend(headers);
        image.extend(body);
        image
    }
}

fn elf_header(kind: u16, machine: u16, header_count: usize) -> Vec<u8> {
    let mut header = vec![0_u8; ELF_HEADER_BYTES];
    header[..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[16..18].copy_from_slice(&kind.to_le_bytes());
    header[18..20].copy_from_slice(&machine.to_le_bytes());
    header[20..24].copy_from_slice(&1_u32.to_le_bytes());
    header[32..40].copy_from_slice(&(ELF_HEADER_BYTES as u64).to_le_bytes());
    header[52..54].copy_from_slice(&(ELF_HEADER_BYTES as u16).to_le_bytes());
    header[54..56].copy_from_slice(&(PROGRAM_HEADER_BYTES as u16).to_le_bytes());
    header[56..58].copy_from_slice(&(header_count as u16).to_le_bytes());
    header
}

fn program_header(
    kind: u32,
    flags: u32,
    offset: usize,
    address: u64,
    file_size: usize,
    extra_memory: u64,
    align: u64,
) -> Vec<u8> {
    let mut header = vec![0_u8; PROGRAM_HEADER_BYTES];
    header[0..4].copy_from_slice(&kind.to_le_bytes());
    header[4..8].copy_from_slice(&flags.to_le_bytes());
    header[8..16].copy_from_slice(&(offset as u64).to_le_bytes());
    header[16..24].copy_from_slice(&address.to_le_bytes());
    header[24..32].copy_from_slice(&address.to_le_bytes());
    header[32..40].copy_from_slice(&(file_size as u64).to_le_bytes());
    header[40..48].copy_from_slice(&(file_size as u64 + extra_memory).to_le_bytes());
    header[48..56].copy_from_slice(&align.to_le_bytes());
    header
}
