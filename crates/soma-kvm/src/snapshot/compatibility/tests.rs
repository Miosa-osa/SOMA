use super::{DeviceExpectation, HostProfile, Incompatibility, check, check_header};
use crate::snapshot::{
    Digest,
    device_state::{DeviceKind, tests as device_fixtures},
    manifest::{
        Architecture, HostCapability, Manifest, PageSize,
        tests::{MEMORY_BYTES, sample_manifest, sample_manifest_with_memory_slots},
    },
    section::{Section, SectionRole},
};

type Mutation = Box<dyn Fn(&mut HostProfile)>;

pub(crate) fn matching_host() -> HostProfile {
    let manifest = sample_manifest();
    let header = manifest.header();
    HostProfile {
        schema_version: 1,
        architecture: Architecture::X86_64,
        page_size: PageSize::FOUR_KIB,
        kvm_api_version: 12,
        capabilities: HostCapability::ALL.to_vec(),
        memory_slots: 32,
        machine_contract: header.machine_contract,
        device_contract: header.device_contract,
        cpu_template: header.cpu_template,
        vcpu_count: 1,
        memory_bytes: MEMORY_BYTES,
        guest_protocol_version: header.guest_protocol_version,
        devices: DeviceKind::ALL.map(|kind| DeviceExpectation {
            kind,
            negotiated_features: device_fixtures::features_for(kind),
            queue_limits: device_fixtures::queue_limits_for(kind),
        }),
    }
}

#[test]
fn a_matching_host_is_compatible_including_after_a_decode_round_trip() {
    let manifest = sample_manifest();
    assert_eq!(check(&matching_host(), &manifest), Ok(()));
    let decoded = Manifest::decode(&manifest.encode()).unwrap();
    assert_eq!(check(&matching_host(), &decoded), Ok(()));
}

#[test]
fn every_header_field_rejects_on_mismatch() {
    let manifest = sample_manifest();
    let other = Digest::of(b"other");
    let cases: Vec<(Mutation, Incompatibility)> = vec![
        (
            Box::new(|host| host.schema_version = 2),
            Incompatibility::SchemaVersion {
                expected: 2,
                actual: 1,
            },
        ),
        (
            Box::new(|host| host.page_size = PageSize::new(8192).unwrap()),
            Incompatibility::PageSize {
                expected: 8192,
                actual: 4096,
            },
        ),
        (
            Box::new(|host| host.memory_bytes = MEMORY_BYTES * 2),
            Incompatibility::MemoryLayout {
                expected: MEMORY_BYTES * 2,
                actual: MEMORY_BYTES,
            },
        ),
        (
            Box::new(|host| host.vcpu_count = 2),
            Incompatibility::VcpuCount {
                expected: 2,
                actual: 1,
            },
        ),
        (
            Box::new(move |host| host.cpu_template = other),
            Incompatibility::CpuTemplate {
                expected: other,
                actual: manifest.header().cpu_template,
            },
        ),
        (
            Box::new(|host| host.kvm_api_version = 11),
            Incompatibility::KvmApiVersion {
                expected: 11,
                actual: 12,
            },
        ),
        (
            Box::new(|host| host.capabilities.retain(|c| *c != HostCapability::IrqFd)),
            Incompatibility::MissingCapability(HostCapability::IrqFd),
        ),
        (
            Box::new(|host| host.memory_slots = 1),
            Incompatibility::MemorySlots {
                required: 2,
                available: 1,
            },
        ),
        (
            Box::new(move |host| host.machine_contract = other),
            Incompatibility::MachineContract {
                expected: other,
                actual: manifest.header().machine_contract,
            },
        ),
        (
            Box::new(move |host| host.device_contract = other),
            Incompatibility::DeviceContract {
                expected: other,
                actual: manifest.header().device_contract,
            },
        ),
        (
            Box::new(|host| host.guest_protocol_version = 9),
            Incompatibility::GuestProtocolVersion {
                expected: 9,
                actual: manifest.header().guest_protocol_version,
            },
        ),
    ];
    for (mutate, expected) in cases {
        let mut host = matching_host();
        mutate(&mut host);
        assert_eq!(check_header(&host, &manifest), Err(expected));
        assert_eq!(check(&host, &manifest), Err(expected));
    }
}

#[test]
fn queue_limit_feature_and_missing_slot_expectations_reject() {
    let manifest = sample_manifest();
    let mut host = matching_host();
    host.devices[4].negotiated_features = 0;
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::FeatureNegotiation {
            slot: 4,
            expected: 0,
            actual: device_fixtures::features_for(DeviceKind::Rng)
        })
    );
    let mut host = matching_host();
    host.devices[3].queue_limits[2] = 128;
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::QueueLimit {
            slot: 3,
            queue: 2,
            expected: 128,
            actual: 64
        })
    );
    let mut host = matching_host();
    host.devices[2].kind = DeviceKind::Rng;
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::NoExpectationForSlot(2))
    );
}

#[test]
fn memory_layout_that_does_not_cover_the_object_rejects() {
    let host = matching_host();
    let manifest = sample_manifest_with_memory_slots(MEMORY_BYTES - 4096);
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::MemoryLayout {
            expected: MEMORY_BYTES,
            actual: MEMORY_BYTES - 4096
        })
    );
    let mut sections = sample_manifest().sections().to_vec();
    sections[0] = Section::new(SectionRole::VmState, vec![0, 0]).unwrap();
    let malformed = Manifest::new(sample_manifest().header().clone(), sections).unwrap();
    assert!(matches!(
        check(&host, &malformed),
        Err(Incompatibility::MalformedVmState(_))
    ));
    let mut sections = sample_manifest().sections().to_vec();
    sections[5] = Section::new(SectionRole::Device0, vec![9]).unwrap();
    let malformed = Manifest::new(sample_manifest().header().clone(), sections).unwrap();
    assert!(matches!(
        check(&host, &malformed),
        Err(Incompatibility::MalformedDevice { slot: 0, .. })
    ));
}
