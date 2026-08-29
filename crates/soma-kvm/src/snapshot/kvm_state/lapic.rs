//! Local APIC register page and multiprocessing state.

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

/// The LAPIC register page exposed by `KVM_GET_LAPIC` is exactly 1 KiB.
pub const LAPIC_LEN: usize = 1024;

#[derive(Clone, Eq, PartialEq)]
pub struct LapicState {
    regs: Box<[u8; LAPIC_LEN]>,
}

impl std::fmt::Debug for LapicState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LapicState(1024 bytes)")
    }
}

impl Default for LapicState {
    fn default() -> Self {
        Self {
            regs: Box::new([0; LAPIC_LEN]),
        }
    }
}

impl LapicState {
    #[must_use]
    pub fn new(regs: [u8; LAPIC_LEN]) -> Self {
        Self {
            regs: Box::new(regs),
        }
    }

    #[must_use]
    pub fn regs(&self) -> &[u8; LAPIC_LEN] {
        &self.regs
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_bytes(self.regs.as_slice());
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let bytes = reader.take(LAPIC_LEN)?;
        let mut regs = Box::new([0_u8; LAPIC_LEN]);
        regs.copy_from_slice(bytes);
        Ok(Self { regs })
    }
}

/// `KVM_GET_MP_STATE` values SOMA accepts in a certified snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MpState {
    #[default]
    Runnable,
    Uninitialized,
    InitReceived,
    Halted,
    SipiReceived,
}

impl MpState {
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::Runnable => 0,
            Self::Uninitialized => 1,
            Self::InitReceived => 2,
            Self::Halted => 3,
            Self::SipiReceived => 4,
        }
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError::UnknownCode`] for any other KVM value.
    pub const fn from_code(code: u32) -> Result<Self, KvmStateError> {
        match code {
            0 => Ok(Self::Runnable),
            1 => Ok(Self::Uninitialized),
            2 => Ok(Self::InitReceived),
            3 => Ok(Self::Halted),
            4 => Ok(Self::SipiReceived),
            _ => Err(KvmStateError::UnknownCode {
                field: "mp_state",
                code,
            }),
        }
    }

    pub(crate) fn write(self, writer: &mut Writer) {
        writer.put_u32(self.code());
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        Self::from_code(reader.u32()?)
    }
}
