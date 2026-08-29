//! Register conversions: `kvm_regs`, `kvm_segment`, `kvm_dtable`, `kvm_sregs`.

use kvm_bindings::{kvm_dtable, kvm_regs, kvm_segment, kvm_sregs};

use super::{BindingError, flag};
use crate::snapshot::kvm_state::{Dtable, Regs, Segment, Sregs};

impl From<kvm_regs> for Regs {
    fn from(regs: kvm_regs) -> Self {
        Self {
            rax: regs.rax,
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rsp: regs.rsp,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            rflags: regs.rflags,
        }
    }
}

impl From<Regs> for kvm_regs {
    fn from(regs: Regs) -> Self {
        Self {
            rax: regs.rax,
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rsp: regs.rsp,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            rflags: regs.rflags,
        }
    }
}

impl TryFrom<kvm_segment> for Segment {
    type Error = BindingError;

    fn try_from(segment: kvm_segment) -> Result<Self, BindingError> {
        let value = Self {
            base: segment.base,
            limit: segment.limit,
            selector: segment.selector,
            type_: segment.type_,
            present: flag("segment.present", segment.present)?,
            dpl: segment.dpl,
            db: flag("segment.db", segment.db)?,
            s: flag("segment.s", segment.s)?,
            l: flag("segment.l", segment.l)?,
            g: flag("segment.g", segment.g)?,
            avl: flag("segment.avl", segment.avl)?,
            unusable: flag("segment.unusable", segment.unusable)?,
        };
        value.validate()?;
        Ok(value)
    }
}

impl From<Segment> for kvm_segment {
    fn from(segment: Segment) -> Self {
        Self {
            base: segment.base,
            limit: segment.limit,
            selector: segment.selector,
            type_: segment.type_,
            present: u8::from(segment.present),
            dpl: segment.dpl,
            db: u8::from(segment.db),
            s: u8::from(segment.s),
            l: u8::from(segment.l),
            g: u8::from(segment.g),
            avl: u8::from(segment.avl),
            unusable: u8::from(segment.unusable),
            padding: 0,
        }
    }
}

impl From<kvm_dtable> for Dtable {
    fn from(table: kvm_dtable) -> Self {
        Self {
            base: table.base,
            limit: table.limit,
        }
    }
}

impl From<Dtable> for kvm_dtable {
    fn from(table: Dtable) -> Self {
        Self {
            base: table.base,
            limit: table.limit,
            padding: [0; 3],
        }
    }
}

impl TryFrom<kvm_sregs> for Sregs {
    type Error = BindingError;

    fn try_from(sregs: kvm_sregs) -> Result<Self, BindingError> {
        Ok(Self {
            cs: sregs.cs.try_into()?,
            ds: sregs.ds.try_into()?,
            es: sregs.es.try_into()?,
            fs: sregs.fs.try_into()?,
            gs: sregs.gs.try_into()?,
            ss: sregs.ss.try_into()?,
            tr: sregs.tr.try_into()?,
            ldt: sregs.ldt.try_into()?,
            gdt: sregs.gdt.into(),
            idt: sregs.idt.into(),
            cr0: sregs.cr0,
            cr2: sregs.cr2,
            cr3: sregs.cr3,
            cr4: sregs.cr4,
            cr8: sregs.cr8,
            efer: sregs.efer,
            apic_base: sregs.apic_base,
            interrupt_bitmap: sregs.interrupt_bitmap,
        })
    }
}

impl From<Sregs> for kvm_sregs {
    fn from(sregs: Sregs) -> Self {
        Self {
            cs: sregs.cs.into(),
            ds: sregs.ds.into(),
            es: sregs.es.into(),
            fs: sregs.fs.into(),
            gs: sregs.gs.into(),
            ss: sregs.ss.into(),
            tr: sregs.tr.into(),
            ldt: sregs.ldt.into(),
            gdt: sregs.gdt.into(),
            idt: sregs.idt.into(),
            cr0: sregs.cr0,
            cr2: sregs.cr2,
            cr3: sregs.cr3,
            cr4: sregs.cr4,
            cr8: sregs.cr8,
            efer: sregs.efer,
            apic_base: sregs.apic_base,
            interrupt_bitmap: sregs.interrupt_bitmap,
        }
    }
}

#[cfg(test)]
mod tests {
    use kvm_bindings::{kvm_regs, kvm_segment, kvm_sregs};

    use super::super::BindingError;
    use crate::snapshot::kvm_state::{Regs, Segment, Sregs};

    #[test]
    fn regs_round_trip_through_kvm_regs() {
        let regs = Regs {
            rip: 0x1000,
            rbx: 0x6000,
            rflags: 2,
            ..Regs::default()
        };
        let raw: kvm_regs = regs.into();
        assert_eq!(raw.rip, 0x1000);
        assert_eq!(Regs::from(raw), regs);
    }

    #[test]
    fn segments_reject_out_of_range_kvm_flags() {
        let raw = kvm_segment {
            present: 2,
            ..kvm_segment::default()
        };
        assert_eq!(
            Segment::try_from(raw),
            Err(BindingError::FlagOutOfRange {
                field: "segment.present",
                value: 2
            })
        );
        let raw = kvm_segment {
            dpl: 4,
            ..kvm_segment::default()
        };
        assert!(matches!(
            Segment::try_from(raw),
            Err(BindingError::State(_))
        ));
        let flat = kvm_segment {
            limit: 0xffff_ffff,
            type_: 11,
            present: 1,
            s: 1,
            db: 1,
            g: 1,
            ..kvm_segment::default()
        };
        let sregs = kvm_sregs {
            cs: flat,
            cr0: 1,
            ..kvm_sregs::default()
        };
        let typed = Sregs::try_from(sregs).unwrap();
        assert!(typed.cs.present && typed.cs.db);
        let back: kvm_sregs = typed.into();
        assert_eq!(back.cs.limit, 0xffff_ffff);
        assert_eq!(back.cr0, 1);
    }
}
