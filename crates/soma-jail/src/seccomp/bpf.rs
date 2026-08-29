//! Hand-assembled classic BPF for `SECCOMP_SET_MODE_FILTER`.
//!
//! The encoder is portable and deterministic: the same policy always produces the same bytes,
//! which the golden tests pin.
//! No libseccomp dependency exists; every instruction is written here.

/// `BPF_LD | BPF_W | BPF_ABS`: load one 32-bit word of `seccomp_data` into the accumulator.
pub const LOAD_WORD: u16 = 0x0020;
/// `BPF_JMP | BPF_JEQ | BPF_K`: jump if the accumulator equals `k`.
pub const JUMP_EQ: u16 = 0x0015;
/// `BPF_JMP | BPF_JSET | BPF_K`: jump if any bit of `k` is set in the accumulator.
pub const JUMP_SET: u16 = 0x0045;
/// `BPF_RET | BPF_K`: return `k`.
pub const RETURN: u16 = 0x0006;

pub const RET_KILL_PROCESS: u32 = 0x8000_0000;
pub const RET_ERRNO: u32 = 0x0005_0000;
pub const RET_ALLOW: u32 = 0x7fff_0000;

/// `AUDIT_ARCH_X86_64`; the profile targets only this architecture.
pub const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;

/// Offsets into `struct seccomp_data`.
pub const DATA_NR: u32 = 0;
pub const DATA_ARCH: u32 = 4;
const DATA_ARGS: u32 = 16;

/// Offset of the low 32 bits of argument `index`.
#[must_use]
pub const fn arg_low(index: u8) -> u32 {
    DATA_ARGS + (index as u32) * 8
}

/// Offset of the high 32 bits of argument `index`.
#[must_use]
pub const fn arg_high(index: u8) -> u32 {
    arg_low(index) + 4
}

/// One `struct sock_filter`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

impl Instruction {
    #[must_use]
    pub const fn load(offset: u32) -> Self {
        Self {
            code: LOAD_WORD,
            jt: 0,
            jf: 0,
            k: offset,
        }
    }

    #[must_use]
    pub const fn jump_eq(k: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: JUMP_EQ,
            jt,
            jf,
            k,
        }
    }

    #[must_use]
    pub const fn jump_set(k: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: JUMP_SET,
            jt,
            jf,
            k,
        }
    }

    #[must_use]
    pub const fn ret(k: u32) -> Self {
        Self {
            code: RETURN,
            jt: 0,
            jf: 0,
            k,
        }
    }

    /// The little-endian `sock_filter` layout: `code`, `jt`, `jf`, `k`.
    #[must_use]
    pub fn bytes(self) -> [u8; 8] {
        let code = self.code.to_le_bytes();
        let k = self.k.to_le_bytes();
        [code[0], code[1], self.jt, self.jf, k[0], k[1], k[2], k[3]]
    }
}

/// A complete assembled filter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilterProgram {
    instructions: Vec<Instruction>,
}

impl FilterProgram {
    pub(super) fn new(instructions: Vec<Instruction>) -> Self {
        Self { instructions }
    }

    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// The exact bytes the kernel receives.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.instructions
            .iter()
            .flat_map(|instruction| instruction.bytes())
            .collect()
    }

    /// FNV-1a over [`Self::bytes`], used by the golden tests and evidence.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        self.bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_bytes_follow_sock_filter_layout() {
        let bytes = Instruction::jump_eq(0xAE80, 3, 7).bytes();
        assert_eq!(bytes, [0x15, 0x00, 3, 7, 0x80, 0xAE, 0x00, 0x00]);
        assert_eq!(
            Instruction::load(DATA_ARCH).bytes(),
            [0x20, 0, 0, 0, 4, 0, 0, 0]
        );
    }

    #[test]
    fn argument_offsets_match_seccomp_data() {
        assert_eq!(arg_low(0), 16);
        assert_eq!(arg_high(0), 20);
        assert_eq!(arg_low(1), 24);
        assert_eq!(arg_low(5), 56);
    }
}
