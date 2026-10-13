use super::{DeviceExpectation, HostProfile, Incompatibility, check, check_header};
use crate::snapshot::{
    Digest,
    device_state::{DeviceKind, tests as device_fixtures},
    manifest::{
        Architecture, HostCapability, Manifest, PageSize, SCHEMA_VERSION,
        tests::{
            MEMORY_BYTES, sample_manifest, sample_manifest_with_mem,
            sample_manifest_with_memory_slots,
        },
    },
    section::{Section, SectionRole},
};

type Mutation = Box<dyn Fn(&mut HostProfile)>;

pub(crate) fn matching_host() -> HostProfile {
    let manifest = sample_manifest();
    let header = manifest.header();
    HostProfile {
        schema_version: SCHEMA_VERSION,
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
        devices: DeviceKind::ALL.map(|kind| {
            let spare = matches!(kind, DeviceKind::Mem | DeviceKind::VolumeBlock);
            (!spare).then(|| DeviceExpectation {
                kind,
                negotiated_features: device_fixtures::features_for(kind),
                queue_limits: device_fixtures::queue_limits_for(kind),
            })
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
            Box::new(|host| host.schema_version = 3),
            Incompatibility::SchemaVersion {
                expected: 3,
                actual: SCHEMA_VERSION,
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
    expectation(&mut host, 4).negotiated_features = 0;
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::FeatureNegotiation {
            slot: 4,
            expected: 0,
            actual: device_fixtures::features_for(DeviceKind::Rng)
        })
    );
    let mut host = matching_host();
    expectation(&mut host, 3).queue_limits[2] = 128;
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
    expectation(&mut host, 2).kind = DeviceKind::Rng;
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::NoExpectationForSlot(2))
    );
    // A host whose Generation declared no network must refuse a snapshot that still carries the
    // device: the captured guest negotiated one, and this machine would not have it.
    let mut host = matching_host();
    host.devices[2] = None;
    assert_eq!(
        check(&host, &manifest),
        Err(Incompatibility::UnexpectedSection(SectionRole::Device2))
    );
}

/// The expectation for one slot, which the matching host has for every slot.
fn expectation(host: &mut HostProfile, slot: usize) -> &mut DeviceExpectation {
    host.devices[slot]
        .as_mut()
        .expect("a matching host expects every slot")
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

/// Rebuilds `manifest` with `section` appended in canonical role order.
fn with_extra_section(manifest: &Manifest, role: SectionRole) -> Manifest {
    let payload = manifest
        .section(SectionRole::Vcpu0)
        .expect("the sample carries the bootstrap processor")
        .payload()
        .to_vec();
    let mut sections = manifest.sections().to_vec();
    sections.push(Section::new(role, payload).unwrap());
    sections.sort_by_key(|section| section.role().code());
    Manifest::new(manifest.header().clone(), sections).unwrap()
}

#[test]
fn the_vcpu_section_set_must_match_the_certified_count() {
    let manifest = sample_manifest();
    assert_eq!(check(&matching_host(), &manifest), Ok(()));
    // A manifest that carries a second processor's state while the header says one is refused,
    // because restoring it would build a machine with a processor the capture never had.
    let surplus = with_extra_section(&manifest, SectionRole::Vcpu1);
    assert_eq!(
        check(&matching_host(), &surplus),
        Err(Incompatibility::UnexpectedSection(SectionRole::Vcpu1))
    );
    // And a header that certifies two processors refuses a manifest carrying only one, because the second processor would resume with no state at all.
    let mut header = manifest.header().clone();
    header.vcpu_count = 2;
    let short = Manifest::new(header, manifest.sections().to_vec()).unwrap();
    let mut two = matching_host();
    two.vcpu_count = 2;
    assert_eq!(
        check(&two, &short),
        Err(Incompatibility::MissingSection(SectionRole::Vcpu1))
    );
}

fn region(slots: u64) -> crate::memory_layout::HotplugRegion {
    use crate::memory_layout::{GuestLayout, HOTPLUG_ALIGN_BYTES, HotplugRegion};
    HotplugRegion::new(
        GuestLayout::new(MEMORY_BYTES).unwrap(),
        slots * HOTPLUG_ALIGN_BYTES,
    )
    .unwrap()
}

fn with_hotplug(region: crate::memory_layout::HotplugRegion) -> Manifest {
    let base = sample_manifest();
    let mut sections = base.sections().to_vec();
    let marker = sections.pop().unwrap();
    sections.push(crate::snapshot::hotplug::section(region).unwrap());
    sections.push(marker);
    Manifest::new(base.header().clone(), sections).unwrap()
}

#[test]
fn a_machine_without_a_region_encodes_and_checks_exactly_as_before() {
    use super::check_hotplug;
    let manifest = sample_manifest();
    assert!(manifest.section(SectionRole::HotplugRegion).is_none());
    assert_eq!(check_hotplug(None, &manifest), Ok(()));
    assert_eq!(check(&matching_host(), &manifest), Ok(()));
}

#[test]
fn the_hotplug_region_must_match_the_declared_geometry_exactly() {
    use super::check_hotplug;
    let (four, eight) = (region(4), region(8));
    let manifest = with_hotplug(four);

    assert_eq!(check_hotplug(Some(four), &manifest), Ok(()));
    // The rest of the manifest is still compatible: the region is additive.
    assert_eq!(check(&matching_host(), &manifest), Ok(()));
    let decoded = Manifest::decode(&manifest.encode()).unwrap();
    assert_eq!(check_hotplug(Some(four), &decoded), Ok(()));

    assert_eq!(
        check_hotplug(Some(eight), &manifest),
        Err(Incompatibility::HotplugRegion {
            expected: Some(eight),
            actual: Some(four),
        })
    );
    assert_eq!(
        check_hotplug(None, &manifest),
        Err(Incompatibility::HotplugRegion {
            expected: None,
            actual: Some(four),
        })
    );
    assert_eq!(
        check_hotplug(Some(four), &sample_manifest()),
        Err(Incompatibility::HotplugRegion {
            expected: Some(four),
            actual: None,
        })
    );
}

#[test]
fn a_malformed_hotplug_record_is_refused() {
    use super::check_hotplug;
    let base = sample_manifest();
    let mut sections = base.sections().to_vec();
    let marker = sections.pop().unwrap();
    sections.push(Section::new(SectionRole::HotplugRegion, vec![0; 3]).unwrap());
    sections.push(marker);
    let manifest = Manifest::new(base.header().clone(), sections).unwrap();
    assert!(matches!(
        check_hotplug(None, &manifest),
        Err(Incompatibility::MalformedHotplug(_))
    ));
}

#[test]
fn the_memory_slot_is_required_exactly_when_the_host_declares_it() {
    let with = sample_manifest_with_mem();
    let without = sample_manifest();
    let mut host = matching_host();
    // The common case: neither side has a memory device.
    assert_eq!(check(&host, &without), Ok(()));
    // The machine was captured resizable and this host builds a fixed-size one: refused, because
    // the captured driver negotiated a device this machine would not have.
    assert_eq!(
        check(&host, &with),
        Err(Incompatibility::UnexpectedSection(SectionRole::Device5))
    );
    // The host declares the device and the snapshot has none: refused the other way.
    host.devices[5] = Some(DeviceExpectation {
        kind: DeviceKind::Mem,
        negotiated_features: device_fixtures::features_for(DeviceKind::Mem),
        queue_limits: device_fixtures::queue_limits_for(DeviceKind::Mem),
    });
    assert_eq!(
        check(&host, &without),
        Err(Incompatibility::MissingSection(SectionRole::Device5))
    );
    assert_eq!(check(&host, &with), Ok(()));
}
