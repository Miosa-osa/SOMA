//! Typed, versioned, fixed-layout encodings of `x86_64` KVM state groups.
//!
//! SOMA owns every byte layout here; no `kvm-bindings` struct is serialized raw.
//! Checked conversions to and from the `kvm-bindings` structs live in `bindings` and
//! compile only on Linux `x86_64` so the later live slice can use them.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub mod bindings;
mod clock;
mod cpu_config;
mod events;
#[cfg(test)]
pub(crate) mod fixtures;
mod fpu;
mod irqchip;
mod lapic;
mod nested;
mod regs;
mod routing;
mod sregs;
#[cfg(test)]
mod tests;
mod vm;

use std::{error::Error, fmt};

pub use clock::{ClockState, PitChannel, PitState};
pub use cpu_config::{
    CpuidEntries, CpuidEntry, MAX_CPUID_ENTRIES, MAX_MSR_ENTRIES, MsrEntries, MsrEntry,
};
pub use events::{ExceptionEvent, InterruptEvent, NmiEvent, SmiEvent, VcpuEvents};
pub use fpu::{Fpu, MAX_XCRS, MAX_XSAVE_BYTES, MIN_XSAVE_BYTES, XcrEntry, Xcrs, XsaveArea};
pub use irqchip::{IOAPIC_PINS, IoapicState, IrqchipState, PicState};
pub use lapic::{LAPIC_LEN, LapicState, MpState};
pub use nested::{NESTED_BLOB_LEN, NestedState};
pub use regs::Regs;
pub use routing::{IrqRoutingEntry, IrqRoutingState, MAX_IRQ_ROUTES, RouteTarget};
pub use sregs::{Dtable, Segment, Sregs};
pub use vm::{MAX_MEMORY_SLOTS, MemorySlot, VmState};

use super::{
    WireError,
    wire::{Reader, Writer},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvmStateError {
    Wire(WireError),
    InvalidField { field: &'static str, value: u64 },
    TooManyEntries { field: &'static str, count: usize },
    DuplicateEntry { field: &'static str, key: u64 },
    UnknownCode { field: &'static str, code: u32 },
    Overlap { field: &'static str },
}

impl fmt::Display for KvmStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "KVM state wire error: {error}"),
            Self::InvalidField { field, value } => {
                write!(formatter, "field {field} has invalid value {value:#x}")
            }
            Self::TooManyEntries { field, count } => {
                write!(formatter, "{count} entries in {field} exceed the bound")
            }
            Self::DuplicateEntry { field, key } => {
                write!(formatter, "duplicate {field} entry {key:#x}")
            }
            Self::UnknownCode { field, code } => {
                write!(formatter, "unknown {field} code {code:#x}")
            }
            Self::Overlap { field } => write!(formatter, "{field} entries overlap"),
        }
    }
}

impl Error for KvmStateError {}

impl From<WireError> for KvmStateError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

pub(crate) fn invalid(field: &'static str, value: impl Into<u64>) -> KvmStateError {
    KvmStateError::InvalidField {
        field,
        value: value.into(),
    }
}

/// Every state group of the single v1 vCPU, in restore order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VcpuState {
    cpuid: CpuidEntries,
    msrs: MsrEntries,
    regs: Regs,
    sregs: Sregs,
    fpu: Fpu,
    xcrs: Xcrs,
    xsave: XsaveArea,
    lapic: LapicState,
    mp_state: MpState,
    events: VcpuEvents,
    nested: Option<NestedState>,
}

/// Builder input for [`VcpuState::new`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VcpuStateParts {
    pub cpuid: CpuidEntries,
    pub msrs: MsrEntries,
    pub regs: Regs,
    pub sregs: Sregs,
    pub fpu: Fpu,
    pub xcrs: Xcrs,
    pub xsave: XsaveArea,
    pub lapic: LapicState,
    pub mp_state: MpState,
    pub events: VcpuEvents,
    pub nested: Option<NestedState>,
}

impl VcpuState {
    /// Validates every group invariant before the state can be encoded.
    ///
    /// # Errors
    ///
    /// Returns [`KvmStateError`] when a segment or event field is out of range.
    pub fn new(parts: VcpuStateParts) -> Result<Self, KvmStateError> {
        parts.sregs.validate()?;
        Ok(Self {
            cpuid: parts.cpuid,
            msrs: parts.msrs,
            regs: parts.regs,
            sregs: parts.sregs,
            fpu: parts.fpu,
            xcrs: parts.xcrs,
            xsave: parts.xsave,
            lapic: parts.lapic,
            mp_state: parts.mp_state,
            events: parts.events,
            nested: parts.nested,
        })
    }

    #[must_use]
    pub const fn cpuid(&self) -> &CpuidEntries {
        &self.cpuid
    }

    #[must_use]
    pub const fn msrs(&self) -> &MsrEntries {
        &self.msrs
    }

    #[must_use]
    pub const fn regs(&self) -> &Regs {
        &self.regs
    }

    #[must_use]
    pub const fn sregs(&self) -> &Sregs {
        &self.sregs
    }

    #[must_use]
    pub const fn fpu(&self) -> &Fpu {
        &self.fpu
    }

    #[must_use]
    pub const fn xcrs(&self) -> &Xcrs {
        &self.xcrs
    }

    #[must_use]
    pub const fn xsave(&self) -> &XsaveArea {
        &self.xsave
    }

    #[must_use]
    pub const fn lapic(&self) -> &LapicState {
        &self.lapic
    }

    #[must_use]
    pub const fn mp_state(&self) -> MpState {
        self.mp_state
    }

    #[must_use]
    pub const fn events(&self) -> &VcpuEvents {
        &self.events
    }

    #[must_use]
    pub const fn nested(&self) -> Option<&NestedState> {
        self.nested.as_ref()
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(8192);
        self.cpuid.write(&mut writer);
        self.msrs.write(&mut writer);
        self.regs.write(&mut writer);
        self.sregs.write(&mut writer);
        self.fpu.write(&mut writer);
        self.xcrs.write(&mut writer);
        self.xsave.write(&mut writer);
        self.lapic.write(&mut writer);
        self.mp_state.write(&mut writer);
        self.events.write(&mut writer);
        writer.put_presence(self.nested.is_some());
        if let Some(nested) = &self.nested {
            nested.write(&mut writer);
        }
        writer.finish()
    }

    /// Decodes one `Vcpu0` section payload.
    ///
    /// # Errors
    ///
    /// Returns [`KvmStateError`] for any malformed, out-of-range, oversized, duplicated,
    /// or trailing input.
    pub fn decode(bytes: &[u8]) -> Result<Self, KvmStateError> {
        let mut reader = Reader::new(bytes);
        let state = Self {
            cpuid: CpuidEntries::read(&mut reader)?,
            msrs: MsrEntries::read(&mut reader)?,
            regs: Regs::read(&mut reader)?,
            sregs: Sregs::read(&mut reader)?,
            fpu: Fpu::read(&mut reader)?,
            xcrs: Xcrs::read(&mut reader)?,
            xsave: XsaveArea::read(&mut reader)?,
            lapic: LapicState::read(&mut reader)?,
            mp_state: MpState::read(&mut reader)?,
            events: VcpuEvents::read(&mut reader)?,
            nested: if reader.presence()? {
                Some(NestedState::read(&mut reader)?)
            } else {
                None
            },
        };
        reader.finish()?;
        Ok(state)
    }
}
