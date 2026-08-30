use std::fs;

use super::{
    artifacts::SnapshotPaths,
    error::{Artifact, SnapshotError},
    installation::inspect,
};
use crate::snapshot::{
    Digest,
    device_state::{BlockState, DeviceSpecific, DeviceState},
    inspection::{ArtifactEvidence, CaptureExpectation, InspectionError},
    manifest::{CandidateId, Manifest, tests::sample_manifest},
    memory::MemoryDescriptor,
    section::{Section, SectionRole},
};

fn fixture() -> (tempfile::TempDir, SnapshotPaths, CaptureExpectation) {
    let scratch = tempfile::tempdir().unwrap();
    let paths = SnapshotPaths::new(scratch.path().join("snapshot"));
    fs::create_dir(paths.directory()).unwrap();
    let memory = vec![0x41; 4096];
    let overlay = vec![0x42; 4096];
    fs::write(paths.memory(), &memory).unwrap();
    fs::write(paths.overlay(), &overlay).unwrap();

    let source = sample_manifest();
    let mut header = source.header().clone();
    header.memory = MemoryDescriptor::new(Digest::of(&memory), 4096, 4096).unwrap();
    let sections = source
        .sections()
        .iter()
        .map(|section| {
            if section.role() != SectionRole::Device1 {
                return section.clone();
            }
            let device = DeviceState::decode_for_slot(1, section.payload()).unwrap();
            let rewritten = DeviceState::new(
                device.kind(),
                device.transport(),
                device.negotiated_features(),
                device.queues().to_vec(),
                DeviceSpecific::Block(BlockState {
                    capacity_sectors: 8,
                    block_size: 4096,
                    image_digest: Digest::of(&overlay),
                }),
            )
            .unwrap();
            Section::new(SectionRole::Device1, rewritten.encode()).unwrap()
        })
        .collect();
    let state = Manifest::new(header, sections).unwrap().encode();
    fs::write(paths.state(), &state).unwrap();
    let expected = CaptureExpectation {
        candidate_id: source.header().candidate_id,
        memory: ArtifactEvidence {
            digest: Digest::of(&memory),
            size: 4096,
        },
        overlay: ArtifactEvidence {
            digest: Digest::of(&overlay),
            size: 4096,
        },
        state: ArtifactEvidence {
            digest: Digest::of(&state),
            size: u64::try_from(state.len()).unwrap(),
        },
    };
    (scratch, paths, expected)
}

#[test]
fn filesystem_inspection_verifies_the_three_published_objects() {
    let (_scratch, paths, expected) = fixture();
    assert_eq!(inspect(&paths, expected).unwrap().state, expected.state);
}

#[test]
fn filesystem_inspection_rejects_identity_corruption_truncation_and_absence() {
    let (_scratch, paths, expected) = fixture();
    let mut foreign = expected;
    foreign.candidate_id = CandidateId::new(*Digest::of(b"foreign candidate").as_bytes()).unwrap();
    assert!(matches!(
        inspect(&paths, foreign),
        Err(SnapshotError::Inspection(
            InspectionError::CandidateMismatch
        ))
    ));

    fs::write(paths.memory(), [0x43; 4096]).unwrap();
    assert!(matches!(
        inspect(&paths, expected),
        Err(SnapshotError::Inspection(InspectionError::MemoryBinding))
    ));

    let (_scratch, paths, expected) = fixture();
    fs::write(paths.state(), b"short").unwrap();
    assert!(inspect(&paths, expected).is_err());

    let (_scratch, paths, expected) = fixture();
    fs::remove_file(paths.overlay()).unwrap();
    assert!(matches!(
        inspect(&paths, expected),
        Err(SnapshotError::Io {
            artifact: Artifact::Overlay,
            operation: "open for inspection",
            ..
        })
    ));
}
