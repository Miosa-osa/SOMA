mod support;

use soma::OciPlatform;
use soma_generation::{
    ArtifactRole, CompileErrorKind,
    generation_manifest::{decode_manifest, encode_manifest},
};
use support::manifest::sample;

#[test]
fn decoder_rejects_truncation_trailing_bytes_and_corruption() {
    let bytes = encode_manifest(&sample()).unwrap();
    for length in 0..bytes.len() {
        assert!(
            decode_manifest(&bytes[..length]).is_err(),
            "prefix {length} accepted"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_manifest(&trailing).is_err());
    let mut magic = bytes.clone();
    magic[0] ^= 1;
    assert!(decode_manifest(&magic).is_err());
    let mut schema = bytes.clone();
    schema[9] = 2;
    assert_eq!(
        decode_manifest(&schema).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
    let mut tag = bytes.clone();
    tag[12] = 3;
    assert!(decode_manifest(&tag).is_err());
    let media = ArtifactRole::ErofsRoot.media_type().as_bytes();
    let position = bytes.windows(media.len()).position(|w| w == media).unwrap();
    let mut wrong_media = bytes.clone();
    wrong_media[position] = b'X';
    assert!(decode_manifest(&wrong_media).is_err());
}

#[test]
fn decoder_rejects_duplicate_descriptors_and_unsupported_platforms() {
    let mut duplicate = sample();
    duplicate.overlay.templates[1].descriptor.digest = duplicate.root.descriptor.digest;
    let bytes = encode_manifest(&duplicate).unwrap();
    assert_eq!(
        decode_manifest(&bytes).unwrap_err().kind(),
        CompileErrorKind::InvalidInput
    );
    let mut unsorted = sample();
    unsorted.overlay.templates.swap(0, 1);
    assert!(encode_manifest(&unsorted).is_err());
    let mut arm = sample();
    arm.source.platform = OciPlatform::linux_arm64();
    assert!(encode_manifest(&arm).is_err());
    let mut nul = sample();
    nul.command_line.push(0);
    assert!(encode_manifest(&nul).is_err());
    let mut wrong_role = sample();
    wrong_role.kernel.descriptor.role = ArtifactRole::GuestAgent;
    assert!(encode_manifest(&wrong_role).is_err());
    let mut long = sample();
    long.guest_agent.build_provenance = "x".repeat(257);
    assert_eq!(
        encode_manifest(&long).unwrap_err().kind(),
        CompileErrorKind::LimitExceeded
    );
}

#[test]
fn the_manifest_layout_versions_match_the_contracts_they_restate() {
    // The Generation manifest and the guest launch page must change together, so the compiler
    // constant is bound to the protocol crate's schema rather than restated by hand.
    assert_eq!(
        soma_generation::contracts::LAUNCH_PAGE_LAYOUT_VERSION,
        soma_guest::LAUNCH_PAGE_SCHEMA_VERSION
    );
    assert_eq!(soma_generation::contracts::MEMORY_SLOT_LAYOUT_VERSION, 1);
    assert_eq!(soma_generation::contracts::REPAIR_POLICY_VERSION, 1);
}
