//! FPU, XCR, XSAVE, LAPIC, MP-state, and event conversions.

use std::os::raw::c_char;

use kvm_bindings::{
    kvm_fpu, kvm_lapic_state, kvm_mp_state, kvm_vcpu_events, kvm_vcpu_events__bindgen_ty_1,
    kvm_vcpu_events__bindgen_ty_2, kvm_vcpu_events__bindgen_ty_3, kvm_vcpu_events__bindgen_ty_4,
    kvm_vcpu_events__bindgen_ty_5, kvm_xcr, kvm_xcrs, kvm_xsave,
};

use super::{BindingError, flag};
use crate::snapshot::kvm_state::{
    ExceptionEvent, Fpu, InterruptEvent, LAPIC_LEN, LapicState, MAX_XCRS, MpState, NmiEvent,
    SmiEvent, VcpuEvents, XcrEntry, Xcrs, XsaveArea,
};

impl From<kvm_fpu> for Fpu {
    fn from(fpu: kvm_fpu) -> Self {
        Self {
            fpr: fpu.fpr,
            fcw: fpu.fcw,
            fsw: fpu.fsw,
            ftwx: fpu.ftwx,
            last_opcode: fpu.last_opcode,
            last_ip: fpu.last_ip,
            last_dp: fpu.last_dp,
            xmm: fpu.xmm,
            mxcsr: fpu.mxcsr,
        }
    }
}

impl From<Fpu> for kvm_fpu {
    fn from(fpu: Fpu) -> Self {
        Self {
            fpr: fpu.fpr,
            fcw: fpu.fcw,
            fsw: fpu.fsw,
            ftwx: fpu.ftwx,
            pad1: 0,
            last_opcode: fpu.last_opcode,
            last_ip: fpu.last_ip,
            last_dp: fpu.last_dp,
            xmm: fpu.xmm,
            mxcsr: fpu.mxcsr,
            pad2: 0,
        }
    }
}

impl TryFrom<kvm_xcrs> for Xcrs {
    type Error = BindingError;

    fn try_from(xcrs: kvm_xcrs) -> Result<Self, BindingError> {
        let count = usize::try_from(xcrs.nr_xcrs).unwrap_or(usize::MAX);
        if count > MAX_XCRS {
            return Err(BindingError::TableTooLarge {
                field: "xcrs",
                count,
            });
        }
        let entries = xcrs.xcrs[..count]
            .iter()
            .map(|entry| XcrEntry {
                index: entry.xcr,
                value: entry.value,
            })
            .collect();
        Ok(Self::new(xcrs.flags, entries)?)
    }
}

impl From<&Xcrs> for kvm_xcrs {
    fn from(xcrs: &Xcrs) -> Self {
        let mut raw = Self {
            flags: xcrs.flags(),
            ..Self::default()
        };
        // `Xcrs::new` bounded the entry count to `MAX_XCRS`, the array length.
        for (slot, entry) in raw.xcrs.iter_mut().zip(xcrs.entries()) {
            *slot = kvm_xcr {
                xcr: entry.index,
                reserved: 0,
                value: entry.value,
            };
        }
        raw.nr_xcrs = u32::try_from(xcrs.entries().len()).unwrap_or(0);
        raw
    }
}

impl From<&kvm_xsave> for XsaveArea {
    fn from(xsave: &kvm_xsave) -> Self {
        let mut bytes = Vec::with_capacity(xsave.region.len() * 4);
        for word in xsave.region {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        // A 4096-byte region satisfies every `XsaveArea::new` bound by construction.
        Self::new(bytes).unwrap_or_else(|_| unreachable!("kvm_xsave region is 4096 bytes"))
    }
}

impl TryFrom<&XsaveArea> for kvm_xsave {
    type Error = BindingError;

    fn try_from(area: &XsaveArea) -> Result<Self, BindingError> {
        let bytes = area.as_bytes();
        let mut raw = Self::default();
        if bytes.len() != raw.region.len() * 4 {
            return Err(BindingError::XsaveLength(bytes.len()));
        }
        let (chunks, _) = bytes.as_chunks::<4>();
        for (word, chunk) in raw.region.iter_mut().zip(chunks) {
            *word = u32::from_le_bytes(*chunk);
        }
        Ok(raw)
    }
}

impl From<&kvm_lapic_state> for LapicState {
    fn from(lapic: &kvm_lapic_state) -> Self {
        let mut regs = [0_u8; LAPIC_LEN];
        for (byte, raw) in regs.iter_mut().zip(lapic.regs) {
            *byte = u8::from_ne_bytes(raw.to_ne_bytes());
        }
        Self::new(regs)
    }
}

impl From<&LapicState> for kvm_lapic_state {
    fn from(lapic: &LapicState) -> Self {
        let mut raw = Self::default();
        for (slot, byte) in raw.regs.iter_mut().zip(lapic.regs()) {
            *slot = c_char::from_ne_bytes([*byte]);
        }
        raw
    }
}

impl TryFrom<kvm_mp_state> for MpState {
    type Error = BindingError;

    fn try_from(state: kvm_mp_state) -> Result<Self, BindingError> {
        Ok(Self::from_code(state.mp_state)?)
    }
}

impl From<MpState> for kvm_mp_state {
    fn from(state: MpState) -> Self {
        Self {
            mp_state: state.code(),
        }
    }
}

impl TryFrom<kvm_vcpu_events> for VcpuEvents {
    type Error = BindingError;

    fn try_from(events: kvm_vcpu_events) -> Result<Self, BindingError> {
        Ok(Self {
            exception: ExceptionEvent {
                injected: flag("exception.injected", events.exception.injected)?,
                nr: events.exception.nr,
                has_error_code: flag("exception.has_error_code", events.exception.has_error_code)?,
                pending: flag("exception.pending", events.exception.pending)?,
                error_code: events.exception.error_code,
            },
            interrupt: InterruptEvent {
                injected: flag("interrupt.injected", events.interrupt.injected)?,
                nr: events.interrupt.nr,
                soft: flag("interrupt.soft", events.interrupt.soft)?,
                shadow: events.interrupt.shadow,
            },
            nmi: NmiEvent {
                injected: flag("nmi.injected", events.nmi.injected)?,
                pending: flag("nmi.pending", events.nmi.pending)?,
                masked: flag("nmi.masked", events.nmi.masked)?,
            },
            sipi_vector: events.sipi_vector,
            flags: events.flags,
            smi: SmiEvent {
                smm: flag("smi.smm", events.smi.smm)?,
                pending: flag("smi.pending", events.smi.pending)?,
                smm_inside_nmi: flag("smi.smm_inside_nmi", events.smi.smm_inside_nmi)?,
                latched_init: events.smi.latched_init,
            },
            triple_fault_pending: flag("triple_fault.pending", events.triple_fault.pending)?,
            exception_has_payload: flag("exception_has_payload", events.exception_has_payload)?,
            exception_payload: events.exception_payload,
        })
    }
}

impl From<VcpuEvents> for kvm_vcpu_events {
    fn from(events: VcpuEvents) -> Self {
        Self {
            exception: kvm_vcpu_events__bindgen_ty_1 {
                injected: u8::from(events.exception.injected),
                nr: events.exception.nr,
                has_error_code: u8::from(events.exception.has_error_code),
                pending: u8::from(events.exception.pending),
                error_code: events.exception.error_code,
            },
            interrupt: kvm_vcpu_events__bindgen_ty_2 {
                injected: u8::from(events.interrupt.injected),
                nr: events.interrupt.nr,
                soft: u8::from(events.interrupt.soft),
                shadow: events.interrupt.shadow,
            },
            nmi: kvm_vcpu_events__bindgen_ty_3 {
                injected: u8::from(events.nmi.injected),
                pending: u8::from(events.nmi.pending),
                masked: u8::from(events.nmi.masked),
                pad: 0,
            },
            sipi_vector: events.sipi_vector,
            flags: events.flags,
            smi: kvm_vcpu_events__bindgen_ty_4 {
                smm: u8::from(events.smi.smm),
                pending: u8::from(events.smi.pending),
                smm_inside_nmi: u8::from(events.smi.smm_inside_nmi),
                latched_init: events.smi.latched_init,
            },
            triple_fault: kvm_vcpu_events__bindgen_ty_5 {
                pending: u8::from(events.triple_fault_pending),
            },
            reserved: [0; 26],
            exception_has_payload: u8::from(events.exception_has_payload),
            exception_payload: events.exception_payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use kvm_bindings::{kvm_lapic_state, kvm_mp_state, kvm_vcpu_events, kvm_xcrs, kvm_xsave};

    use super::super::BindingError;
    use crate::snapshot::kvm_state::{LapicState, MpState, VcpuEvents, Xcrs, XsaveArea};

    #[test]
    fn xcrs_lapic_and_xsave_round_trip() {
        let mut raw = kvm_xcrs {
            nr_xcrs: 1,
            ..kvm_xcrs::default()
        };
        raw.xcrs[0].value = 7;
        let typed = Xcrs::try_from(raw).unwrap();
        assert_eq!(typed.entries()[0].value, 7);
        assert_eq!(kvm_xcrs::from(&typed).nr_xcrs, 1);
        raw.nr_xcrs = 17;
        assert!(matches!(
            Xcrs::try_from(raw),
            Err(BindingError::TableTooLarge { .. })
        ));

        let mut lapic = kvm_lapic_state::default();
        lapic.regs[1023] = -1;
        let typed = LapicState::from(&lapic);
        assert_eq!(typed.regs()[1023], 0xff);
        assert_eq!(kvm_lapic_state::from(&typed).regs[1023], -1);

        let mut xsave = kvm_xsave::default();
        xsave.region[0] = 0x0403_0201;
        let typed = XsaveArea::from(&xsave);
        assert_eq!(&typed.as_bytes()[..4], &[1, 2, 3, 4]);
        assert_eq!(kvm_xsave::try_from(&typed).unwrap().region[0], 0x0403_0201);
        let long = XsaveArea::new(vec![0; 8192]).unwrap();
        assert_eq!(
            kvm_xsave::try_from(&long).unwrap_err(),
            BindingError::XsaveLength(8192)
        );
    }

    #[test]
    fn mp_state_and_events_are_checked() {
        assert_eq!(
            MpState::try_from(kvm_mp_state { mp_state: 3 }),
            Ok(MpState::Halted)
        );
        assert!(MpState::try_from(kvm_mp_state { mp_state: 99 }).is_err());
        let mut raw = kvm_vcpu_events::default();
        raw.nmi.masked = 1;
        let typed = VcpuEvents::try_from(raw).unwrap();
        assert!(typed.nmi.masked);
        assert_eq!(kvm_vcpu_events::from(typed).nmi.masked, 1);
        raw.smi.smm = 5;
        assert_eq!(
            VcpuEvents::try_from(raw),
            Err(BindingError::FlagOutOfRange {
                field: "smi.smm",
                value: 5
            })
        );
    }
}
