//! Every capacity gate rejects by name, rejections roll back, and the atlas ladder holds.

mod atlas;

use atlas::{GIB, MIB, atlas_profile, atlas_shape};
use soma_hostd::{
    Admission, Gate, InstanceShape, MeasuredOverhead, SingleNode, WorkloadClass, estimate,
};

#[test]
fn the_capacity_ladder_of_the_visual_atlas_reproduces() {
    let medium = estimate(&atlas_profile(16, 2, 32, 4), &atlas_shape()).expect("estimate");
    assert_eq!(medium.memory, 49, "28,672 MiB / 576 MiB");
    assert_eq!(medium.cpu_strict, 14);
    assert_eq!(medium.cpu_overcommitted, 56, "14 units at 4:1");
    assert_eq!(medium.safe_count, 49);
    assert_eq!(medium.binding, Gate::GuaranteedMemory);
    for (threads, reserved, total, reserved_gib, strict, memory) in [
        (4, 1, 8, 2, 3, 10),
        (8, 2, 16, 3, 6, 23),
        (40, 4, 128, 12, 36, 206),
        (80, 8, 256, 24, 72, 412),
        (160, 16, 512, 48, 144, 824),
    ] {
        let row = estimate(
            &atlas_profile(threads, reserved, total, reserved_gib),
            &atlas_shape(),
        )
        .expect("estimate");
        assert_eq!(
            (row.cpu_strict, row.memory),
            (strict, memory),
            "{threads} threads"
        );
    }
    let two_gib = InstanceShape {
        guest_memory_bytes: 2 * GIB,
        ..atlas_shape()
    };
    assert_eq!(
        estimate(&atlas_profile(80, 8, 256, 24), &two_gib)
            .expect("estimate")
            .memory,
        112
    );
    let small_host = atlas_profile(80, 8, 25, 5);
    for (guest_mib, memory_bound) in [(1024, 18), (512, 35), (256, 64), (128, 106)] {
        let shape = InstanceShape {
            guest_memory_bytes: guest_mib * MIB,
            workload: WorkloadClass::Build,
            ..atlas_shape()
        };
        let row = estimate(&small_host, &shape).expect("estimate");
        assert_eq!(row.memory, memory_bound);
        assert_eq!(row.cpu_strict, 72);
        assert_eq!(row.safe_count, memory_bound.min(72));
    }
}

#[test]
fn reservations_admit_exactly_the_memory_bound_then_reject_with_evidence() {
    let admission = Admission::new(atlas_profile(16, 2, 32, 4), SingleNode);
    let mut reservations = Vec::new();
    loop {
        match admission.reserve(&atlas_shape()) {
            Ok(reservation) => reservations.push(reservation),
            Err(rejection) => {
                assert_eq!(reservations.len(), 49);
                assert_eq!(rejection.gate, Gate::GuaranteedMemory);
                assert_eq!(rejection.requested, 576 * MIB);
                assert_eq!(rejection.committed, 49 * 576 * MIB);
                assert_eq!(rejection.limit, 28 * GIB);
                assert!(rejection.available() < 576 * MIB);
                break;
            }
        }
    }
    let usage = admission.usage();
    assert_eq!(usage.residents, 49);
    assert_eq!(usage.launches, 49);
    for mut reservation in reservations.drain(..) {
        admission.launched(&mut reservation);
        let slot = admission.begin_cleanup(&reservation).expect("cleanup slot");
        admission.release(reservation, Some(slot));
    }
    let empty = admission.usage();
    assert_eq!(empty.residents, 0);
    assert_eq!(empty.guaranteed_bytes, 0);
    assert_eq!(empty.cpu_milli_units, 0);
    assert_eq!(empty.cleanups, 0);
    assert_eq!(empty.node_cpu, vec![0]);
}

#[test]
fn measured_overhead_inputs_are_labelled() {
    assert!(
        MeasuredOverhead::ATLAS_PLACEHOLDER
            .evidence
            .contains("placeholder")
    );
    assert!(
        MeasuredOverhead::PVH_BOOT_SINGLE_SAMPLE
            .evidence
            .contains("not certified")
    );
    assert_eq!(
        MeasuredOverhead::PVH_BOOT_SINGLE_SAMPLE.bytes_per_instance,
        4 * MIB
    );
    let mut measured = atlas_profile(16, 2, 32, 4);
    measured.memory.overhead = MeasuredOverhead::PVH_BOOT_SINGLE_SAMPLE;
    let row = estimate(&measured, &atlas_shape()).expect("estimate");
    assert_eq!(row.memory, 28 * 1024 / 516);
    assert_ne!(measured.digest(), atlas_profile(16, 2, 32, 4).digest());
}
