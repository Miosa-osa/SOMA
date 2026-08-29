//! Every capacity gate rejects by name, rejections roll back, and the atlas ladder holds.

mod atlas;

use atlas::{GIB, MIB, atlas_profile, atlas_shape, atlas_valid, certified, valid};
use soma_hostd::{
    Admission, Gate, InstanceShape, MeasuredOverhead, Ratio, SingleNode, WorkloadClass, estimate,
};

#[test]
fn the_capacity_ladder_of_the_visual_atlas_reproduces() {
    let medium = estimate(&atlas_profile(16, 2, 32, 4), &atlas_valid()).expect("estimate");
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
            &atlas_valid(),
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
        estimate(&atlas_profile(80, 8, 256, 24), &valid(two_gib))
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
        let row = estimate(&small_host, &valid(shape)).expect("estimate");
        assert_eq!(row.memory, memory_bound);
        assert_eq!(row.cpu_strict, 72);
        assert_eq!(row.safe_count, memory_bound.min(72));
    }
}

#[test]
fn the_three_to_one_row_of_the_ladder_admits_forty_two_and_not_forty_one() {
    let mut profile = atlas_profile(16, 2, 32, 4).into_profile();
    profile.overcommit.api_waiting = Ratio {
        vcpus: 3,
        threads: 1,
    };
    let profile = certified(profile);
    let row = estimate(&profile, &atlas_valid()).expect("estimate");
    assert_eq!(
        row.cpu_overcommitted, 42,
        "the atlas ladder reads 42 at 3:1 on 14 admissible units"
    );
    assert_eq!(row.safe_count, 42);
    assert_eq!(row.binding, Gate::CpuUnits);
    let admission = Admission::new(profile, SingleNode);
    let mut held = Vec::new();
    loop {
        match admission.reserve(&atlas_valid()) {
            Ok(reservation) => held.push(reservation),
            Err(rejection) => {
                assert_eq!(rejection.gate, Gate::CpuUnits);
                break;
            }
        }
    }
    assert_eq!(
        held.len(),
        42,
        "the ratio is applied once to the census, not once per Instance"
    );
    assert_eq!(admission.usage().vcpus_by_class[0], 42);
    assert_eq!(admission.usage().cpu_milli_units, 14_000);
}

#[test]
fn reservations_admit_exactly_the_memory_bound_then_reject_with_evidence() {
    let admission = Admission::new(atlas_profile(16, 2, 32, 4), SingleNode);
    let mut reservations = Vec::new();
    loop {
        match admission.reserve(&atlas_valid()) {
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
        admission
            .begin_cleanup(&mut reservation)
            .expect("cleanup slot");
        admission.release(reservation);
    }
    let empty = admission.usage();
    assert_eq!(empty.residents, 0);
    assert_eq!(empty.guaranteed_bytes, 0);
    assert_eq!(empty.cpu_milli_units, 0);
    assert_eq!(empty.cleanups, 0);
    assert_eq!(empty.node_cpu, vec![0]);
}

#[test]
fn the_estimate_names_the_burst_limit_a_reservation_will_actually_hit() {
    for (row_bound, gate, adjust) in [
        (10_u64, Gate::RunnableVcpus, 0_u64),
        (10, Gate::DirtyMemory, 5 * GIB),
    ] {
        let mut profile = atlas_profile(16, 2, 32, 4).into_profile();
        if adjust == 0 {
            profile.limits.runnable_vcpus = 10;
        } else {
            profile.limits.dirty_memory_bytes = adjust;
        }
        let profile = certified(profile);
        let row = estimate(&profile, &atlas_valid()).expect("estimate");
        assert_eq!(row.safe_count, row_bound, "{gate:?}");
        assert_eq!(row.binding, gate);
        let admission = Admission::new(profile, SingleNode);
        let mut held = Vec::new();
        let rejection = loop {
            match admission.reserve(&atlas_valid()) {
                Ok(reservation) => held.push(reservation),
                Err(rejection) => break rejection,
            }
        };
        assert_eq!(
            u64::try_from(held.len()).expect("count"),
            row_bound,
            "the estimate and the admitted count agree"
        );
        assert_eq!(rejection.gate, gate);
    }
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
    let mut measured = atlas_profile(16, 2, 32, 4).into_profile();
    measured.memory.overhead = MeasuredOverhead::PVH_BOOT_SINGLE_SAMPLE;
    let row = estimate(&certified(measured), &atlas_valid()).expect("estimate");
    assert_eq!(row.memory, 28 * 1024 / 516);
    assert_ne!(
        measured.digest(),
        atlas_profile(16, 2, 32, 4).profile().digest()
    );
}
