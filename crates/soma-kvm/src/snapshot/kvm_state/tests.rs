use super::{
    ClockState, CpuidEntries, CpuidEntry, IrqRoutingState, IrqchipState, KvmStateError, MemorySlot,
    MpState, MsrEntries, MsrEntry, NESTED_BLOB_LEN, NestedState, PitState, VcpuState,
    VcpuStateParts, VmState, XcrEntry, Xcrs, XsaveArea,
    fixtures::{
        sample_clock, sample_irqchip, sample_routing, sample_sregs, sample_vcpu, sample_vm,
    },
};
use crate::snapshot::WireError;

#[test]
fn vcpu_state_round_trips_with_and_without_nested_state() {
    let plain = sample_vcpu(None);
    let bytes = plain.encode();
    assert_eq!(VcpuState::decode(&bytes), Ok(plain.clone()));
    assert_eq!(plain.regs().rip, 0x0100_0000);
    assert_eq!(plain.cpuid().entries().len(), 2);
    assert_eq!(plain.msrs().entries()[1].value, 1234);
    assert_eq!(plain.sregs().cr3, 0x1000);
    assert_eq!(plain.fpu().fcw, 0x37f);
    assert_eq!(plain.xcrs().entries()[0].value, 7);
    assert_eq!(plain.xsave().as_bytes()[0], 0x7f);
    assert_eq!(plain.lapic().regs()[0x20], 1);
    assert_eq!(plain.mp_state(), MpState::Runnable);
    assert_eq!(plain.events().exception.nr, 14);
    assert!(plain.nested().is_none());

    let mut vmcs12 = Box::new([0_u8; NESTED_BLOB_LEN]);
    vmcs12[4095] = 9;
    let nested = sample_vcpu(Some(NestedState::Vmx {
        flags: 1,
        vmxon_pa: 0x1000,
        vmcs12_pa: 0x2000,
        smm_flags: 0,
        hdr_flags: 0,
        preemption_timer_deadline: 0,
        vmcs12,
        shadow_vmcs12: Box::new([0; NESTED_BLOB_LEN]),
    }));
    let bytes = nested.encode();
    assert_eq!(VcpuState::decode(&bytes), Ok(nested.clone()));
    assert!(nested.nested().is_some_and(|n| n.flags() == 1));
    let svm = sample_vcpu(Some(NestedState::Svm {
        flags: 0,
        vmcb_pa: 0x9000,
        vmcb12: Box::new([1; NESTED_BLOB_LEN]),
    }));
    assert_eq!(VcpuState::decode(&svm.encode()), Ok(svm));
}

#[test]
fn vcpu_state_rejects_every_short_prefix_trailing_byte_and_bad_presence() {
    let bytes = sample_vcpu(None).encode();
    for length in 0..bytes.len() {
        assert!(VcpuState::decode(&bytes[..length]).is_err());
    }
    let mut extended = bytes.clone();
    extended.push(0);
    assert_eq!(
        VcpuState::decode(&extended),
        Err(KvmStateError::Wire(WireError::TrailingBytes(1)))
    );
    let mut bad_presence = bytes;
    let last = bad_presence.len() - 1;
    bad_presence[last] = 2;
    assert_eq!(
        VcpuState::decode(&bad_presence),
        Err(KvmStateError::Wire(WireError::InvalidPresence(2)))
    );
}

#[test]
fn constructors_reject_bounds_duplicates_and_ranges() {
    assert!(matches!(
        CpuidEntries::new(vec![CpuidEntry::default(); 257]),
        Err(KvmStateError::TooManyEntries { field: "cpuid", .. })
    ));
    assert!(matches!(
        MsrEntries::new(vec![MsrEntry::default(); 2]),
        Err(KvmStateError::DuplicateEntry {
            field: "msrs",
            key: 0
        })
    ));
    assert!(matches!(
        Xcrs::new(0, vec![XcrEntry { index: 1, value: 0 }; 2]),
        Err(KvmStateError::DuplicateEntry { field: "xcrs", .. })
    ));
    assert!(XsaveArea::new(vec![0; 4095]).is_err());
    assert!(XsaveArea::new(vec![0; 65540]).is_err());
    let mut sregs = sample_sregs();
    sregs.gs.dpl = 4;
    assert_eq!(
        VcpuState::new(VcpuStateParts {
            sregs,
            ..parts_of(&sample_vcpu(None))
        }),
        Err(KvmStateError::InvalidField {
            field: "segment.dpl",
            value: 4
        })
    );
    assert!(MpState::from_code(5).is_err());
}

fn parts_of(state: &VcpuState) -> VcpuStateParts {
    VcpuStateParts {
        cpuid: state.cpuid().clone(),
        msrs: state.msrs().clone(),
        regs: *state.regs(),
        sregs: *state.sregs(),
        fpu: *state.fpu(),
        xcrs: state.xcrs().clone(),
        xsave: state.xsave().clone(),
        lapic: state.lapic().clone(),
        mp_state: state.mp_state(),
        events: *state.events(),
        nested: state.nested().cloned(),
    }
}

#[test]
fn vm_irqchip_routing_clock_and_pit_round_trip_and_reject_trailing_bytes() {
    let vm = sample_vm(256 << 20);
    assert_eq!(VmState::decode(&vm.encode()), Ok(vm.clone()));
    assert_eq!(vm.total_bytes(), 256 << 20);
    assert_eq!(vm.tss_address(), 0xfffb_d000);
    assert_eq!(vm.identity_map_address(), 0xfffb_c000);
    let irqchip = sample_irqchip();
    assert_eq!(IrqchipState::decode(&irqchip.encode()), Ok(irqchip));
    let routing = sample_routing();
    assert_eq!(
        IrqRoutingState::decode(&routing.encode()),
        Ok(routing.clone())
    );
    assert_eq!(routing.entries().len(), 5);
    let clock = sample_clock();
    assert_eq!(ClockState::decode(&clock.encode()), Ok(clock));
    let mut pit = PitState::default();
    pit.channels[0].count_load_time = -1;
    pit.flags = 1;
    assert_eq!(PitState::decode(&pit.encode()), Ok(pit));

    for bytes in [
        vm.encode(),
        irqchip.encode(),
        routing.encode(),
        clock.encode(),
        pit.encode(),
    ] {
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(VmState::decode(&extended).is_err());
        assert!(IrqchipState::decode(&extended).is_err());
        assert!(IrqRoutingState::decode(&extended).is_err());
        assert!(ClockState::decode(&extended).is_err());
        assert!(PitState::decode(&extended).is_err());
        for length in 0..bytes.len() {
            let _ = VmState::decode(&bytes[..length]);
            let _ = IrqchipState::decode(&bytes[..length]);
            let _ = IrqRoutingState::decode(&bytes[..length]);
            let _ = ClockState::decode(&bytes[..length]);
            let _ = PitState::decode(&bytes[..length]);
        }
    }
}

#[test]
fn vm_layout_rejects_overlap_duplicates_overflow_and_unknown_codes() {
    let slot = |slot, guest_address, size| MemorySlot {
        slot,
        guest_address,
        size,
        memory_offset: 0,
    };
    assert!(matches!(
        VmState::new(vec![slot(0, 0, 0x2000), slot(1, 0x1000, 0x1000)], 0, 0),
        Err(KvmStateError::Overlap { .. })
    ));
    assert!(matches!(
        VmState::new(vec![slot(0, 0, 0x1000), slot(0, 0x1000, 0x1000)], 0, 0),
        Err(KvmStateError::DuplicateEntry { .. })
    ));
    assert!(matches!(
        VmState::new(vec![slot(0, u64::MAX, 2)], 0, 0),
        Err(KvmStateError::InvalidField { .. })
    ));
    assert!(VmState::new(vec![], 0, 0).is_err());
    let mut routing = sample_routing().encode();
    routing[10] = 9;
    assert_eq!(
        IrqRoutingState::decode(&routing),
        Err(KvmStateError::UnknownCode {
            field: "irq_routing.target",
            code: 9
        })
    );
    let mut clock = sample_clock().encode();
    clock[11] = 1;
    assert!(ClockState::decode(&clock).is_err());
}

#[test]
fn frozen_clock_keeps_the_value_and_drops_the_realtime_pairing() {
    // A restore must install exactly the clock the guest paused with.
    // Passing the captured KVM_CLOCK_REALTIME pairing through KVM_SET_CLOCK
    // makes the kernel advance the guest's monotonic clock by the whole
    // capture-to-restore wall interval - the netdev-watchdog/cleanup defect
    // in the 2026-08-30 restore-stage-timeline evidence.
    let captured = ClockState {
        clock: 987_654_321,
        // KVM_CLOCK_TSC_STABLE | KVM_CLOCK_REALTIME | KVM_CLOCK_HOST_TSC,
        // exactly what KVM_GET_CLOCK reports on a modern host.
        flags: 2 | 4 | 8,
        realtime: 1_756_500_000_000_000_000,
        host_tsc: 42_000_000_000,
    };

    let installed = captured.frozen();

    assert_eq!(installed.clock, captured.clock);
    assert_eq!(installed.flags, 0);
    assert_eq!(installed.realtime, 0);
    assert_eq!(installed.host_tsc, 0);
    // The captured record itself is untouched by value semantics: the
    // snapshot remains a faithful read of KVM_GET_CLOCK.
    assert_eq!(captured.flags, 2 | 4 | 8);
}
