//! One set decides the machine: the device set the compiled manifest declares.
//!
//! `GenerationManifest::device_set()` derives a machine's device set from the manifest's own
//! declared fields, and the verifier compares the command line and the device contract against
//! that derived set (`generation/verify/machine.rs`, `Incompatibility::CommandLine` and
//! `ContractStatement`). A compiler that composed either of those from the Template alone
//! published a manifest the compiler itself could never certify, so this test drives
//! `ManifestParts::build`, the compiler's own field rules, under the persistence policy and then
//! runs the verifier over what it produced.

use soma::{MachineShape, OciDigest, OciImage, OciPlatform, WorkloadIdentity};

use super::ManifestParts;
use crate::{
    CompilerProfile, LifetimeLimits, StartupBehavior, TemplateImage, TemplateRevision,
    generation::{
        artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
        manifest::{GenerationManifest, OverlayTemplate, RootBinding, TreeBinding},
        verify::machine::require_machine,
    },
};

const MIB: u64 = 1024 * 1024;
/// The volume device the command line names for a persistent Generation.
const VOLUME_TOKEN: &str = "soma.volume=/dev/vdc";

fn digest(fill: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([fill; 32])
}

fn descriptor(role: ArtifactRole, fill: u8, size: u64) -> ArtifactDescriptor {
    ArtifactDescriptor {
        role,
        digest: digest(fill),
        size,
    }
}

/// A Template revision and a workload identity for one profile: the writable class the profile
/// builds, the isolated network policy, and a shape inside the profile's bounds.
fn inputs(profile: &CompilerProfile) -> (TemplateRevision, WorkloadIdentity) {
    let platform = OciPlatform::linux_amd64();
    let manifest_digest =
        OciDigest::parse(format!("sha256:{}", "1".repeat(64))).expect("a canonical digest");
    let image = TemplateImage::new(
        OciImage::parse("example.test/fixture:amd64").expect("a canonical image reference"),
        manifest_digest.clone(),
        platform.clone(),
    );
    let revision = TemplateRevision::new(
        image,
        MachineShape::new(8, 8 * 1024, profile.overlay_capacities[0] / MIB)
            .expect("a shape inside the profile's bounds"),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(3600).expect("a lifetime inside the bound"),
        profile.policy_version,
    )
    .expect("a revision the profile accepts");
    (
        revision,
        WorkloadIdentity::new(manifest_digest, platform, None),
    )
}

/// The compiler's own field rules over one revision and one profile.
fn build(
    revision: &TemplateRevision,
    workload: &WorkloadIdentity,
    profile: &CompilerProfile,
) -> GenerationManifest {
    let capacity = revision.writable_storage_bytes();
    ManifestParts {
        template: revision,
        workload,
        profile,
        tree: TreeBinding {
            digest: digest(0x02),
            size: 3_678_098,
        },
        root: RootBinding {
            descriptor: descriptor(ArtifactRole::ErofsRoot, 0x03, 1_200_001_024),
            uuid: [0x11; 16],
            format_profile: "erofs-fixed-profile-v1".to_owned(),
            formatter_digest: digest(0x04),
            formatter_revision: "1.7.1".to_owned(),
            builder_environment_digest: digest(0x05),
        },
        templates: vec![OverlayTemplate {
            capacity,
            descriptor: descriptor(ArtifactRole::OverlayTemplate, 0x06, capacity),
        }],
        kernel: descriptor(ArtifactRole::Kernel, 0x07, 9_000_000),
        initramfs: descriptor(ArtifactRole::Initramfs, 0x08, 2_000_000),
        guest_agent: descriptor(ArtifactRole::GuestAgent, 0x09, 1_500_000),
        config_digest: digest(0x0a),
        early_init_digest: digest(0x0b),
        initramfs_layout_version: 1,
    }
    .build()
    .expect("the compiler builds this revision")
}

/// The profile's verdict on one compiled manifest, or the invariant it refused.
fn verdict(manifest: &GenerationManifest, profile: &CompilerProfile) -> Option<String> {
    require_machine(manifest, profile)
        .err()
        .map(|error| format!("{:?}", error.incompatibility()))
}

#[test]
fn a_policy_four_compile_satisfies_the_profile_it_was_built_under() {
    let profile = CompilerProfile::from_policy_version(4).expect("policy 4 exists");
    let (revision, workload) = inputs(&profile);
    let manifest = build(&revision, &workload, &profile);

    assert!(
        profile.persistence.enabled,
        "policy 4 is the persistence policy"
    );
    assert!(
        manifest.device_set().volume(),
        "the manifest's own fields declare the durable volume"
    );
    assert_eq!(
        verdict(&manifest, &profile),
        None,
        "a policy 4 compile must satisfy the profile it was built under"
    );
    assert_eq!(
        revision.device_set(),
        manifest.device_set(),
        "the machine the compiler builds and the machine the manifest declares are one"
    );
    let line = String::from_utf8_lossy(&manifest.command_line);
    assert!(
        line.contains(VOLUME_TOKEN),
        "the line the compiler binds names the volume device: {line}"
    );
}

#[test]
fn a_policy_three_compile_declares_no_volume() {
    let profile = CompilerProfile::from_policy_version(3).expect("policy 3 exists");
    let (revision, workload) = inputs(&profile);
    let manifest = build(&revision, &workload, &profile);

    assert!(
        !profile.persistence.enabled,
        "policy 3 predates the persistence binding"
    );
    assert!(
        !manifest.device_set().volume(),
        "only the persistence policy declares the volume"
    );
    let line = String::from_utf8_lossy(&manifest.command_line);
    assert!(
        !line.contains("soma.volume"),
        "a policy 3 line names no volume device: {line}"
    );
    assert_eq!(
        verdict(&manifest, &profile),
        None,
        "policy 3 still verifies"
    );
}
