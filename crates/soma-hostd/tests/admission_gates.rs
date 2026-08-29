//! Every capacity gate rejects by its name and a rejection rolls back every dimension.

mod atlas;

use atlas::{GIB, atlas_profile, atlas_shape, atlas_valid, certified, valid};
use soma_hostd::{
    Admission, CertifiedProfile, Gate, InstanceShape, MemoryClass, NumaRejection, SingleNode,
    WorkloadClass,
};

fn gated_profile() -> CertifiedProfile {
    let mut profile = atlas_profile(16, 2, 32, 4).into_profile();
    profile.storage.private_budget_bytes = 10 * GIB;
    profile.network.units = 100;
    profile.process.processes = 5;
    profile.process.descriptors = 200;
    profile.limits.resident_instances = 4;
    profile.limits.concurrent_launches = 2;
    profile.limits.runnable_vcpus = 6;
    profile.limits.dirty_memory_bytes = 3 * GIB;
    profile.limits.cleanup_slots = 1;
    profile.validate().expect("profile")
}

#[test]
fn every_gate_rejects_by_name_and_a_rejection_leaves_usage_unchanged() {
    let admission = Admission::new(gated_profile(), SingleNode);
    let shape = |f: fn(InstanceShape) -> InstanceShape| f(atlas_shape());
    let cases: [(Gate, InstanceShape); 10] = [
        (
            Gate::CpuUnits,
            shape(|s| InstanceShape {
                vcpus: 57,
                workload: WorkloadClass::Build,
                ..s
            }),
        ),
        (
            Gate::HostMemory,
            shape(|s| InstanceShape {
                guest_memory_bytes: 29 * GIB,
                ..s
            }),
        ),
        (
            Gate::ElasticMemory,
            shape(|s| InstanceShape {
                guest_memory_bytes: 8 * GIB,
                memory_class: MemoryClass::Elastic {
                    expected_resident_bytes: 8 * GIB,
                },
                ..s
            }),
        ),
        (
            Gate::PrivateStorage,
            shape(|s| InstanceShape {
                private_storage_bytes: 11 * GIB,
                ..s
            }),
        ),
        (
            Gate::NetworkInventory,
            shape(|s| InstanceShape {
                network_units: 101,
                ..s
            }),
        ),
        (
            Gate::DescriptorLimit,
            shape(|s| InstanceShape {
                descriptors: 201,
                ..s
            }),
        ),
        (
            Gate::RunnableVcpus,
            shape(|s| InstanceShape { vcpus: 7, ..s }),
        ),
        (
            Gate::DirtyMemory,
            shape(|s| InstanceShape {
                guest_memory_bytes: 4 * GIB,
                ..s
            }),
        ),
        (
            Gate::Arithmetic,
            shape(|s| InstanceShape {
                guest_memory_bytes: u64::MAX,
                ..s
            }),
        ),
        (
            Gate::CpuUnits,
            shape(|s| InstanceShape {
                vcpus: 15,
                workload: WorkloadClass::Build,
                ..s
            }),
        ),
    ];
    for (gate, shape) in cases {
        let before = admission.usage();
        let rejection = admission.reserve(&valid(shape)).expect_err("gate");
        assert_eq!(rejection.gate, gate, "{shape:?}");
        assert_eq!(admission.usage(), before, "{gate:?} rolled back");
    }
}

#[test]
fn launch_resident_cleanup_and_process_limits_gate_in_sequence() {
    let admission = Admission::new(gated_profile(), SingleNode);
    let profile = gated_profile();
    let first = admission.reserve(&atlas_valid()).expect("first");
    let second = admission.reserve(&atlas_valid()).expect("second");
    assert_eq!(
        admission
            .reserve(&atlas_valid())
            .expect_err("launches")
            .gate,
        Gate::ConcurrentLaunches
    );
    let mut first = first;
    let mut second = second;
    admission.launched(&mut first);
    admission.launched(&mut second);
    let third = admission.reserve(&atlas_valid()).expect("third");
    let mut third = third;
    admission.launched(&mut third);
    let fourth = admission.reserve(&atlas_valid()).expect("fourth");
    assert_eq!(
        admission
            .reserve(&atlas_valid())
            .expect_err("residents")
            .gate,
        Gate::OperatorSafetyLimit
    );
    let mut fourth = fourth;
    admission.launched(&mut fourth);
    admission.begin_cleanup(&mut first).expect("slot");
    assert_eq!(
        admission
            .begin_cleanup(&mut second)
            .expect_err("cleanup")
            .gate,
        Gate::CleanupSlots
    );
    admission.release(first);
    admission.release(second);
    admission.release(third);
    admission.release(fourth);
    let mut tight = profile.into_profile();
    tight.process.processes = 1;
    let admission = Admission::new(certified(tight), SingleNode);
    let held = admission.reserve(&atlas_valid()).expect("one process");
    assert_eq!(
        admission
            .reserve(&atlas_valid())
            .expect_err("processes")
            .gate,
        Gate::ProcessLimit
    );
    admission.release(held);
}

#[test]
fn a_cleanup_slot_is_owned_by_the_reservation_that_took_it() {
    let admission = Admission::new(gated_profile(), SingleNode);
    let mut first = admission.reserve(&atlas_valid()).expect("first");
    admission.launched(&mut first);
    let mut second = admission.reserve(&atlas_valid()).expect("second");
    admission.launched(&mut second);

    admission.begin_cleanup(&mut first).expect("slot");
    assert!(first.holds_cleanup_slot());
    assert_eq!(admission.usage().cleanups, 1);
    admission.begin_cleanup(&mut first).expect("idempotent");
    assert_eq!(
        admission.usage().cleanups,
        1,
        "a second begin takes no second slot"
    );
    assert_eq!(
        admission
            .begin_cleanup(&mut second)
            .expect_err("every slot is busy")
            .gate,
        Gate::CleanupSlots
    );
    assert!(!second.holds_cleanup_slot());

    admission.release(second);
    assert_eq!(
        admission.usage().cleanups,
        1,
        "a reservation that holds no slot returns none"
    );
    admission.release(first);
    assert_eq!(
        admission.usage().cleanups,
        0,
        "the slot returns exactly once"
    );
    let mut third = admission.reserve(&atlas_valid()).expect("third");
    admission
        .begin_cleanup(&mut third)
        .expect("the slot is available again");
    admission.release(third);
    assert_eq!(admission.usage().cleanups, 0);
}

#[test]
fn placement_rejects_multi_node_hosts_and_fragmentation_by_name() {
    let mut two_nodes = atlas_profile(16, 2, 32, 4).into_profile();
    two_nodes.cpu.numa_nodes = 2;
    let admission = Admission::new(certified(two_nodes), SingleNode);
    let rejection = admission.reserve(&atlas_valid()).expect_err("multi-node");
    assert_eq!(rejection.gate, Gate::NumaFit);
    assert_eq!(admission.usage().residents, 0);
    let demand = soma_hostd::NodeDemand {
        cpu_milli_units: 8_000,
        memory_bytes: 8 * GIB,
    };
    assert_eq!(
        soma_hostd::NumaPlacement::place(&SingleNode, demand, &[]),
        Err(NumaRejection::NoNodes)
    );
}
