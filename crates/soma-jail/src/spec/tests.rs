//! Proofs that [`JailSpec::validate`] refuses every unsafe field, and that [`LeafName`]
//! refuses every name that could escape the cgroup root.
//!
//! Each case starts from one complete specification and changes exactly one field, so a
//! rejection can only be attributed to the field under test.

use super::*;
use crate::manifest::DescriptorRole;

fn spec() -> JailSpec {
    JailSpec {
        identity: Identity {
            uid: 60_001,
            gid: 60_001,
        },
        leaf: LeafName::new("vmm-0001").expect("valid leaf"),
        limits: CgroupLimits {
            memory_max_bytes: 256 << 20,
            cpu_max: CpuMax {
                quota_us: 100_000,
                period_us: 100_000,
            },
            pids_max: 8,
            io_max: None,
        },
        rlimits: Rlimits {
            nofile: 64,
            nproc: 8,
            fsize_bytes: 1 << 30,
            address_space_bytes: None,
        },
        manifest: DescriptorManifest::new(vec![DescriptorRole::Control]).expect("manifest"),
        phase: Phase::Startup,
    }
}

#[test]
fn accepts_a_complete_specification() {
    assert_eq!(spec().validate(), Ok(()));
}

#[test]
fn rejects_root_and_overflow_identities() {
    for uid in [0, 65_534, 65_535] {
        let mut bad = spec();
        bad.identity.uid = uid;
        assert_eq!(bad.validate(), Err(SpecError::Identity));
    }
}

#[test]
fn rejects_unsafe_leaf_names() {
    for name in ["", ".", "..", "../x", "a/b", ".hidden", "with space"] {
        assert_eq!(LeafName::new(name), Err(SpecError::LeafName), "{name:?}");
    }
    assert!(LeafName::new("vmm-1.a_b").is_ok());
}

#[test]
fn rejects_zero_and_inverted_limits() {
    let mut bad = spec();
    bad.limits.memory_max_bytes = 0;
    assert_eq!(bad.validate(), Err(SpecError::MemoryMaxZero));
    let mut bad = spec();
    bad.limits.cpu_max.quota_us = bad.limits.cpu_max.period_us + 1;
    assert_eq!(bad.validate(), Err(SpecError::CpuMaxInvalid));
    let mut bad = spec();
    bad.limits.pids_max = 0;
    assert_eq!(bad.validate(), Err(SpecError::PidsMaxZero));
    let mut bad = spec();
    bad.limits.io_max = Some(IoMax {
        major: 8,
        minor: 0,
        read_bytes_per_second: 0,
        write_bytes_per_second: 0,
        read_iops: 0,
        write_iops: 0,
    });
    assert_eq!(bad.validate(), Err(SpecError::IoMaxInvalid));
}

#[test]
fn nofile_must_cover_every_sealed_slot() {
    let mut bad = spec();
    bad.rlimits.nofile = 4;
    assert_eq!(
        bad.validate(),
        Err(SpecError::NofileTooSmall { required: 5 })
    );
    bad.rlimits.nofile = 5;
    assert_eq!(bad.validate(), Ok(()));
}

#[test]
fn only_the_startup_phase_launches() {
    let mut bad = spec();
    bad.phase = Phase::SteadyState;
    assert_eq!(bad.validate(), Err(SpecError::UnlaunchablePhase));
}
