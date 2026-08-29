//! Deterministic state fixtures shared by codec, manifest, and compatibility tests.

use super::{
    ClockState, CpuidEntries, CpuidEntry, Dtable, ExceptionEvent, Fpu, IoapicState,
    IrqRoutingEntry, IrqRoutingState, IrqchipState, LapicState, MemorySlot, MpState, MsrEntries,
    MsrEntry, NestedState, PicState, Regs, RouteTarget, Segment, Sregs, VcpuEvents, VcpuState,
    VcpuStateParts, VmState, XcrEntry, Xcrs, XsaveArea,
};

fn flat_segment(type_: u8) -> Segment {
    Segment {
        base: 0,
        limit: 0xffff_ffff,
        selector: 0x10,
        type_,
        present: true,
        dpl: 0,
        db: true,
        s: true,
        l: false,
        g: true,
        avl: false,
        unusable: false,
    }
}

pub(crate) fn sample_sregs() -> Sregs {
    Sregs {
        cs: flat_segment(11),
        ds: flat_segment(3),
        es: flat_segment(3),
        fs: flat_segment(3),
        gs: flat_segment(3),
        ss: flat_segment(3),
        tr: Segment {
            limit: 0x67,
            type_: 11,
            s: false,
            db: false,
            g: false,
            ..flat_segment(11)
        },
        ldt: Segment {
            unusable: true,
            ..Segment::default()
        },
        gdt: Dtable {
            base: 0x500,
            limit: 0x1f,
        },
        idt: Dtable::default(),
        cr0: 0x8005_0033,
        cr2: 0,
        cr3: 0x1000,
        cr4: 0x6e0,
        cr8: 0,
        efer: 0x500,
        apic_base: 0xfee0_0900,
        interrupt_bitmap: [0; 4],
    }
}

pub(crate) fn sample_vcpu(nested: Option<NestedState>) -> VcpuState {
    let mut lapic = [0_u8; 1024];
    lapic[0x20] = 1;
    let mut xsave = vec![0_u8; 4096];
    xsave[0] = 0x7f;
    VcpuState::new(VcpuStateParts {
        cpuid: CpuidEntries::new(vec![
            CpuidEntry {
                function: 0,
                eax: 0xd,
                ebx: 0x756e_6547,
                ..CpuidEntry::default()
            },
            CpuidEntry {
                function: 7,
                index: 0,
                flags: 1,
                ..CpuidEntry::default()
            },
        ])
        .unwrap(),
        msrs: MsrEntries::new(vec![
            MsrEntry {
                index: 0xc000_0080,
                value: 0x500,
            },
            MsrEntry {
                index: 0x10,
                value: 1234,
            },
        ])
        .unwrap(),
        regs: Regs {
            rip: 0x0100_0000,
            rbx: 0x6000,
            rflags: 0x2,
            ..Regs::default()
        },
        sregs: sample_sregs(),
        fpu: Fpu {
            fcw: 0x37f,
            mxcsr: 0x1f80,
            ..Fpu::default()
        },
        xcrs: Xcrs::new(0, vec![XcrEntry { index: 0, value: 7 }]).unwrap(),
        xsave: XsaveArea::new(xsave).unwrap(),
        lapic: LapicState::new(lapic),
        mp_state: MpState::Runnable,
        events: VcpuEvents {
            exception: ExceptionEvent {
                nr: 14,
                ..ExceptionEvent::default()
            },
            flags: 0x3,
            ..VcpuEvents::default()
        },
        nested,
    })
    .unwrap()
}

pub(crate) fn sample_vm(memory_bytes: u64) -> VmState {
    VmState::new(
        vec![
            MemorySlot {
                slot: 0,
                guest_address: 0,
                size: 0xa_0000,
                memory_offset: 0,
            },
            MemorySlot {
                slot: 1,
                guest_address: 0x10_0000,
                size: memory_bytes - 0xa_0000,
                memory_offset: 0xa_0000,
            },
        ],
        0xfffb_d000,
        0xfffb_c000,
    )
    .unwrap()
}

pub(crate) fn sample_irqchip() -> IrqchipState {
    let mut ioapic = IoapicState {
        base_address: 0xfec0_0000,
        ..IoapicState::default()
    };
    ioapic.redirection[5] = 0x1_0000_0025;
    IrqchipState {
        master: PicState::from_array([0, 0, 0xff, 0, 0, 0x08, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0xf8]),
        slave: PicState::from_array([0, 0, 0xff, 0, 0, 0x70, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0xde]),
        ioapic,
    }
}

pub(crate) fn sample_routing() -> IrqRoutingState {
    IrqRoutingState::new(
        (5..10)
            .map(|gsi| IrqRoutingEntry {
                gsi,
                flags: 0,
                target: RouteTarget::Irqchip {
                    irqchip: 2,
                    pin: gsi,
                },
            })
            .collect(),
    )
    .unwrap()
}

pub(crate) fn sample_clock() -> ClockState {
    ClockState {
        clock: 1_234_567_890,
        flags: 2,
        realtime: 0,
        host_tsc: 0,
    }
}
