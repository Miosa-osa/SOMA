use super::{
    Architecture, CandidateId, HostCapability, HostRequirements, HostRequirementsError, MAGIC,
    Manifest, ManifestError, ManifestHeader, PageSize, SCHEMA_VERSION,
};
use crate::snapshot::{
    Digest, WireError,
    device_state::{DeviceKind, tests::sample as sample_device},
    kvm_state::fixtures::{sample_clock, sample_irqchip, sample_routing, sample_vcpu, sample_vm},
    memory::MemoryDescriptor,
    section::{MAX_SECTIONS, Section, SectionError, SectionRole},
};

pub(crate) const MEMORY_BYTES: u64 = 256 << 20;

pub(crate) fn sample_header() -> ManifestHeader {
    ManifestHeader {
        architecture: Architecture::X86_64,
        page_size: PageSize::FOUR_KIB,
        candidate_id: CandidateId::new(*Digest::of(b"candidate").as_bytes()).unwrap(),
        machine_contract: Digest::of(b"machine-contract-v1"),
        device_contract: Digest::of(b"device-contract-v1"),
        cpu_template: Digest::of(b"cpu-template-v1"),
        host: HostRequirements::new(
            12,
            vec![
                HostCapability::UserMemory,
                HostCapability::IrqChip,
                HostCapability::IrqFd,
                HostCapability::IoEventFd,
                HostCapability::ImmediateExit,
            ],
            2,
        )
        .unwrap(),
        memory: MemoryDescriptor::new(Digest::of(b"memory.raw"), MEMORY_BYTES, 4096).unwrap(),
        vcpu_count: 1,
        guest_protocol_version: 1,
    }
}

pub(crate) fn sample_manifest_with_memory_slots(covered: u64) -> Manifest {
    let section = |role, payload| Section::new(role, payload).unwrap();
    let mut sections = vec![
        section(SectionRole::VmState, sample_vm(covered).encode()),
        section(SectionRole::Vcpu0, sample_vcpu(None).encode()),
        section(SectionRole::Irqchip, sample_irqchip().encode()),
        section(SectionRole::IrqRouting, sample_routing().encode()),
        section(SectionRole::KvmClock, sample_clock().encode()),
    ];
    for (kind, role) in DeviceKind::ALL.into_iter().zip([
        SectionRole::Device0,
        SectionRole::Device1,
        SectionRole::Device2,
        SectionRole::Device3,
        SectionRole::Device4,
    ]) {
        sections.push(section(role, sample_device(kind).encode()));
    }
    sections.push(section(
        SectionRole::RepairPointMarker,
        b"repair-point-v1".to_vec(),
    ));
    Manifest::new(sample_header(), sections).unwrap()
}

pub(crate) fn sample_manifest() -> Manifest {
    sample_manifest_with_memory_slots(MEMORY_BYTES)
}

#[test]
fn golden_header_bytes_and_whole_manifest_digest_are_stable() {
    let manifest = sample_manifest();
    let bytes = manifest.encode();
    let mut expected_prefix = Vec::new();
    expected_prefix.extend_from_slice(&MAGIC);
    expected_prefix.extend_from_slice(&SCHEMA_VERSION.to_be_bytes());
    expected_prefix.extend_from_slice(&[0, 1]);
    expected_prefix.extend_from_slice(&4096_u32.to_be_bytes());
    expected_prefix.extend_from_slice(Digest::of(b"candidate").as_bytes());
    expected_prefix.extend_from_slice(Digest::of(b"machine-contract-v1").as_bytes());
    expected_prefix.extend_from_slice(Digest::of(b"device-contract-v1").as_bytes());
    expected_prefix.extend_from_slice(Digest::of(b"cpu-template-v1").as_bytes());
    expected_prefix.extend_from_slice(&[0, 0, 0, 12, 0, 5, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 2]);
    expected_prefix.extend_from_slice(Digest::of(b"memory.raw").as_bytes());
    expected_prefix.extend_from_slice(&MEMORY_BYTES.to_be_bytes());
    expected_prefix.extend_from_slice(&[0, 1, 0, 1, 0, 11]);
    assert_eq!(&bytes[..expected_prefix.len()], &expected_prefix[..]);
    assert_eq!(bytes.len(), GOLDEN_LEN);
    assert_eq!(Digest::of(&bytes).to_string(), GOLDEN_SHA256);
    assert_eq!(Manifest::decode(&bytes), Ok(manifest));
}

const GOLDEN_LEN: usize = 7678;
const GOLDEN_SHA256: &str = "50fc3c834601d527f0f4a17e46e68bfdf21c638e218cfbdfd9b9692d66467544";

#[test]
fn every_single_byte_flip_is_rejected_or_visibly_different() {
    let manifest = sample_manifest();
    let bytes = manifest.encode();
    for index in 0..bytes.len() {
        let mut corrupted = bytes.clone();
        corrupted[index] ^= 0x01;
        let decoded = Manifest::decode(&corrupted);
        assert_ne!(
            decoded,
            Ok(manifest.clone()),
            "flip at byte {index} was accepted"
        );
    }
}

#[test]
fn every_truncation_length_is_a_typed_error() {
    let bytes = sample_manifest().encode();
    for length in 0..bytes.len() {
        assert!(
            Manifest::decode(&bytes[..length]).is_err(),
            "prefix {length} accepted"
        );
    }
    let mut extended = bytes;
    extended.push(0);
    assert_eq!(
        Manifest::decode(&extended),
        Err(ManifestError::Wire(WireError::TrailingBytes(1)))
    );
}

#[test]
fn rejects_bad_magic_schema_architecture_and_zero_fields() {
    let bytes = sample_manifest().encode();
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert_eq!(Manifest::decode(&bad_magic), Err(ManifestError::BadMagic));
    let mut schema = bytes.clone();
    schema[9] = 3;
    assert_eq!(
        Manifest::decode(&schema),
        Err(ManifestError::UnsupportedSchemaVersion(3))
    );
    let mut retired_schema = bytes.clone();
    retired_schema[9] = 1;
    assert_eq!(
        Manifest::decode(&retired_schema),
        Err(ManifestError::UnsupportedSchemaVersion(1))
    );
    let mut arch = bytes.clone();
    arch[11] = 7;
    assert_eq!(
        Manifest::decode(&arch),
        Err(ManifestError::UnknownArchitecture(7))
    );
    let mut page = bytes.clone();
    page[15] = 1;
    assert_eq!(
        Manifest::decode(&page),
        Err(ManifestError::InvalidPageSize(4097))
    );
    let mut generation = bytes.clone();
    generation[16..48].fill(0);
    assert_eq!(
        Manifest::decode(&generation),
        Err(ManifestError::ZeroCandidateId)
    );
    let capability_count = 16 + 32 * 4 + 4;
    let mut capabilities = bytes.clone();
    capabilities[capability_count + 1] = 0xff;
    assert_eq!(
        Manifest::decode(&capabilities),
        Err(ManifestError::Wire(WireError::LengthExceedsBound {
            length: 0xff,
            bound: 32
        }))
    );
    let mut unknown_capability = bytes.clone();
    unknown_capability[capability_count + 3] = 0x7f;
    assert_eq!(
        Manifest::decode(&unknown_capability),
        Err(ManifestError::HostRequirements(
            HostRequirementsError::UnknownCapability(0x7f)
        ))
    );
    let mut vcpus = bytes;
    let vcpu_index = capability_count + 2 + 10 + 2 + 40;
    vcpus[vcpu_index + 1] = 0;
    assert_eq!(Manifest::decode(&vcpus), Err(ManifestError::ZeroVcpuCount));
}

#[test]
fn rejects_absent_required_duplicate_and_reordered_sections() {
    let manifest = sample_manifest();
    let mut without_vcpu = manifest.sections().to_vec();
    without_vcpu.remove(1);
    assert_eq!(
        Manifest::new(sample_header(), without_vcpu).unwrap_err(),
        ManifestError::MissingRequiredSection(SectionRole::Vcpu0)
    );
    let mut duplicated = manifest.sections().to_vec();
    duplicated.insert(1, duplicated[1].clone());
    assert_eq!(
        Manifest::new(sample_header(), duplicated).unwrap_err(),
        ManifestError::Section(SectionError::RoleOrder {
            previous: 2,
            next: 2
        })
    );
    let mut reordered = manifest.sections().to_vec();
    reordered.swap(0, 1);
    assert!(matches!(
        Manifest::new(sample_header(), reordered),
        Err(ManifestError::Section(SectionError::RoleOrder { .. }))
    ));
    let too_many = vec![manifest.sections()[0].clone(); usize::from(MAX_SECTIONS) + 1];
    assert_eq!(
        Manifest::new(sample_header(), too_many).unwrap_err(),
        ManifestError::TooManySections(MAX_SECTIONS + 1)
    );
    assert!(manifest.section(SectionRole::Pit).is_none());
    assert!(manifest.section(SectionRole::RepairPointMarker).is_some());
}

#[test]
fn absurd_section_length_is_rejected_before_any_allocation() {
    let manifest = sample_manifest();
    let bytes = manifest.encode();
    let header_len = bytes.len() - sections_len(&manifest);
    let mut hostile = bytes[..header_len].to_vec();
    hostile.extend_from_slice(&[0, 1, 0, 1, 1, 0xff, 0xff, 0xff, 0xff]);
    hostile.extend_from_slice(&[0; 32]);
    assert_eq!(
        Manifest::decode(&hostile),
        Err(ManifestError::Section(SectionError::Wire(
            WireError::LengthExceedsBound {
                length: u64::from(u32::MAX),
                bound: 1 << 20
            }
        )))
    );
    let mut too_many = bytes[..header_len - 2].to_vec();
    too_many.extend_from_slice(&[0xff, 0xff]);
    assert_eq!(
        Manifest::decode(&too_many),
        Err(ManifestError::TooManySections(0xffff))
    );
}

fn sections_len(manifest: &Manifest) -> usize {
    manifest
        .sections()
        .iter()
        .map(|section| 41 + section.payload().len())
        .sum()
}

#[test]
fn unknown_non_critical_section_is_skipped_and_unknown_critical_is_rejected() {
    let manifest = sample_manifest();
    let bytes = manifest.encode();
    let header_len = bytes.len() - sections_len(&manifest);
    let count_index = header_len - 2;
    let mut extended = bytes.clone();
    extended[count_index + 1] += 1;
    extended.extend_from_slice(&[0x70, 0x00, 0, 1, 0, 0, 0, 0, 3]);
    extended.extend_from_slice(Digest::of(&[1, 2, 3]).as_bytes());
    extended.extend_from_slice(&[1, 2, 3]);
    assert_eq!(Manifest::decode(&extended), Ok(manifest));
    let flags_index = extended.len() - 3 - 32 - 4 - 1;
    extended[flags_index] = 1;
    assert_eq!(
        Manifest::decode(&extended),
        Err(ManifestError::Section(SectionError::UnknownCriticalRole(
            0x7000
        )))
    );
}
