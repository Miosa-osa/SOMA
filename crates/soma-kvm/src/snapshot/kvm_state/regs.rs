//! General-purpose register encoding: the eighteen architectural u64 values, in order.

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

/// General-purpose registers: 18 x u64 in architectural order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Regs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl Regs {
    pub const ENCODED_LEN: usize = 18 * 8;

    const fn as_array(&self) -> [u64; 18] {
        [
            self.rax,
            self.rbx,
            self.rcx,
            self.rdx,
            self.rsi,
            self.rdi,
            self.rsp,
            self.rbp,
            self.r8,
            self.r9,
            self.r10,
            self.r11,
            self.r12,
            self.r13,
            self.r14,
            self.r15,
            self.rip,
            self.rflags,
        ]
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        for value in self.as_array() {
            writer.put_u64(value);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let mut values = [0_u64; 18];
        for value in &mut values {
            *value = reader.u64()?;
        }
        let [
            rax,
            rbx,
            rcx,
            rdx,
            rsi,
            rdi,
            rsp,
            rbp,
            r8,
            r9,
            r10,
            r11,
            r12,
            r13,
            r14,
            r15,
            rip,
            rflags,
        ] = values;
        Ok(Self {
            rax,
            rbx,
            rcx,
            rdx,
            rsi,
            rdi,
            rsp,
            rbp,
            r8,
            r9,
            r10,
            r11,
            r12,
            r13,
            r14,
            r15,
            rip,
            rflags,
        })
    }
}
