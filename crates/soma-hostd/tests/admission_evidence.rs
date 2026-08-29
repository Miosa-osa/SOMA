//! A capacity rejection names one gate, in one comparable dimension, and never aborts.

mod atlas;

use atlas::{GIB, MIB, atlas_profile, atlas_shape, atlas_valid, certified, valid};
use soma_hostd::{Admission, Gate, InstanceShape, MemoryClass, SingleNode, WorkloadClass};

#[test]
fn overflowing_capacity_arithmetic_is_a_typed_rejection_and_never_a_panic() {
    let mut profile = atlas_profile(16, 2, 32, 4).into_profile();
    profile.overcommit.build = soma_hostd::Ratio {
        vcpus: 1,
        threads: u32::MAX,
    };
    let profile = certified(profile);
    let huge = valid(InstanceShape {
        vcpus: u32::MAX,
        workload: WorkloadClass::Build,
        ..atlas_shape()
    });
    assert_eq!(
        soma_hostd::estimate(&profile, &huge)
            .expect_err("overflow")
            .gate,
        Gate::Arithmetic
    );
    let admission = Admission::new(profile, SingleNode);
    let before = admission.usage();
    assert_eq!(
        admission.reserve(&huge).expect_err("overflow").gate,
        Gate::Arithmetic
    );
    assert_eq!(admission.usage(), before, "an overflow commits nothing");

    let mut zero_ratio = atlas_profile(16, 2, 32, 4).into_profile();
    zero_ratio.overcommit.api_waiting = soma_hostd::Ratio {
        vcpus: 0,
        threads: 1,
    };
    assert_eq!(
        zero_ratio.validate(),
        Err(soma_hostd::ProfileError::ZeroRatio),
        "a zero-sided ratio can never be certified into an admission"
    );
    assert_eq!(
        InstanceShape {
            vcpus: 0,
            ..atlas_shape()
        }
        .validate(),
        Err(soma_hostd::ShapeError::NoVcpus),
        "a shape with no vCPU can never be validated into a reservation"
    );
    assert_eq!(
        InstanceShape {
            guest_memory_bytes: 512 << 20,
            memory_class: MemoryClass::Elastic {
                expected_resident_bytes: 4 * GIB,
            },
            ..atlas_shape()
        }
        .validate(),
        Err(soma_hostd::ShapeError::ElasticAboveGuest),
        "an over-promised elastic set can never be accounted"
    );
}

#[test]
fn a_rejection_names_the_pool_it_refused_in_one_comparable_dimension() {
    let admission = Admission::new(atlas_profile(16, 2, 32, 4), SingleNode);
    while admission.reserve(&atlas_valid()).is_ok() {}
    assert_eq!(admission.usage().residents, 49);
    let elastic = valid(InstanceShape {
        memory_class: MemoryClass::Elastic {
            expected_resident_bytes: 400 * MIB,
        },
        ..atlas_shape()
    });
    let rejection = admission.reserve(&elastic).expect_err("host memory");
    assert_eq!(
        rejection.gate,
        Gate::HostMemory,
        "an elastic request refused by total host memory never names another pool"
    );
    assert_eq!(rejection.limit, 28 * GIB);
    assert_eq!(
        admission.usage().elastic_bytes,
        0,
        "the elastic budget holds nothing"
    );

    let mut two_nodes = atlas_profile(16, 2, 32, 4).into_profile();
    two_nodes.cpu.numa_nodes = 2;
    let admission = Admission::new(certified(two_nodes), SingleNode);
    let rejection = admission.reserve(&atlas_valid()).expect_err("topology");
    assert_eq!(rejection.gate, Gate::NumaFit);
    assert_eq!(
        (rejection.requested, rejection.committed, rejection.limit),
        (2, 0, 1),
        "the three numbers are node counts, not a mix of units"
    );
    assert_eq!(rejection.available(), 1);
}
