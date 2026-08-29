//! Pending exception, interrupt, NMI, SMI, and triple-fault state (`KVM_GET_VCPU_EVENTS`).

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExceptionEvent {
    pub injected: bool,
    pub nr: u8,
    pub has_error_code: bool,
    pub pending: bool,
    pub error_code: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InterruptEvent {
    pub injected: bool,
    pub nr: u8,
    pub soft: bool,
    /// Interrupt shadow bitmask (`KVM_X86_SHADOW_INT_*`).
    pub shadow: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NmiEvent {
    pub injected: bool,
    pub pending: bool,
    pub masked: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SmiEvent {
    pub smm: bool,
    pub pending: bool,
    pub smm_inside_nmi: bool,
    pub latched_init: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuEvents {
    pub exception: ExceptionEvent,
    pub interrupt: InterruptEvent,
    pub nmi: NmiEvent,
    pub sipi_vector: u32,
    pub flags: u32,
    pub smi: SmiEvent,
    pub triple_fault_pending: bool,
    pub exception_has_payload: bool,
    pub exception_payload: u64,
}

impl VcpuEvents {
    pub const ENCODED_LEN: usize = 8 + 4 + 3 + 4 + 4 + 4 + 1 + 1 + 8;

    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_presence(self.exception.injected);
        writer.put_u8(self.exception.nr);
        writer.put_presence(self.exception.has_error_code);
        writer.put_presence(self.exception.pending);
        writer.put_u32(self.exception.error_code);
        writer.put_presence(self.interrupt.injected);
        writer.put_u8(self.interrupt.nr);
        writer.put_presence(self.interrupt.soft);
        writer.put_u8(self.interrupt.shadow);
        writer.put_presence(self.nmi.injected);
        writer.put_presence(self.nmi.pending);
        writer.put_presence(self.nmi.masked);
        writer.put_u32(self.sipi_vector);
        writer.put_u32(self.flags);
        writer.put_presence(self.smi.smm);
        writer.put_presence(self.smi.pending);
        writer.put_presence(self.smi.smm_inside_nmi);
        writer.put_u8(self.smi.latched_init);
        writer.put_presence(self.triple_fault_pending);
        writer.put_presence(self.exception_has_payload);
        writer.put_u64(self.exception_payload);
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let exception = ExceptionEvent {
            injected: reader.presence()?,
            nr: reader.u8()?,
            has_error_code: reader.presence()?,
            pending: reader.presence()?,
            error_code: reader.u32()?,
        };
        let interrupt = InterruptEvent {
            injected: reader.presence()?,
            nr: reader.u8()?,
            soft: reader.presence()?,
            shadow: reader.u8()?,
        };
        let nmi = NmiEvent {
            injected: reader.presence()?,
            pending: reader.presence()?,
            masked: reader.presence()?,
        };
        let sipi_vector = reader.u32()?;
        let flags = reader.u32()?;
        let smi = SmiEvent {
            smm: reader.presence()?,
            pending: reader.presence()?,
            smm_inside_nmi: reader.presence()?,
            latched_init: reader.u8()?,
        };
        Ok(Self {
            exception,
            interrupt,
            nmi,
            sipi_vector,
            flags,
            smi,
            triple_fault_pending: reader.presence()?,
            exception_has_payload: reader.presence()?,
            exception_payload: reader.u64()?,
        })
    }
}
