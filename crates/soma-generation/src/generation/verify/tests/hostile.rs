//! Hostile manifest bytes: truncation, bit mutation, network bindings, and probe semantics.
//!
//! No input may panic, and no accepted input may smuggle a non-canonical encoding past the
//! decoder or an incompatible field past the profile check.

use soma::NetworkPolicy;

use crate::generation::{
    contracts,
    manifest::{GenerationManifest, decode_candidate, encode_candidate, fixture},
    template::{MAX_WORKLOAD_PROBE_BYTES, NetworkPolicyClass, network_policy_digest},
};

use super::{Incompatibility, profile, rejected, require_profile};

#[test]
fn the_network_class_and_the_canonical_policy_digest_must_name_one_policy() {
    let isolated = network_policy_digest(&NetworkPolicy::isolated()).unwrap();
    let runtime = network_policy_digest(&NetworkPolicy::runtime_default()).unwrap();
    assert_ne!(isolated, runtime);

    let mut manifest = fixture::profile_v1();
    manifest.template.network_policy_class = NetworkPolicyClass::RuntimeDefault;
    manifest.template.network_policy_digest = runtime;
    assert!(require_profile(&manifest, &profile()).is_ok());

    rejected(Incompatibility::NetworkPolicy, |m| {
        m.template.network_policy_class = NetworkPolicyClass::RuntimeDefault;
    });
    rejected(Incompatibility::NetworkPolicy, |m| {
        m.template.network_policy_class = NetworkPolicyClass::Explicit;
    });
    rejected(Incompatibility::NetworkPolicy, |m| {
        m.template.network_policy_digest = runtime;
    });
}

#[test]
fn an_explicit_workload_probe_must_name_one_absolute_control_free_executable() {
    let mut manifest = fixture::profile_v1();
    manifest.template.workload_probe = Some(b"/usr/local/bin/node --version".to_vec());
    assert!(require_profile(&manifest, &profile()).is_ok());

    for probe in [
        Vec::new(),
        b"node --version".to_vec(),
        b"/bin/sh\n-c".to_vec(),
        b"/bin/sh\t".to_vec(),
        vec![b'/'; MAX_WORKLOAD_PROBE_BYTES + 1],
    ] {
        rejected(Incompatibility::WorkloadProbe, move |m| {
            m.template.workload_probe = Some(probe);
        });
    }
}

#[test]
fn every_truncation_of_a_canonical_manifest_is_rejected_without_panicking() {
    let bytes = encode_candidate(&fixture::profile_v1()).expect("canonical bytes");

    for length in 0..bytes.len() {
        assert!(
            decode_candidate(&bytes[..length]).is_err(),
            "a truncation of {length} bytes decoded"
        );
    }
    assert!(decode_candidate(&bytes).is_ok());
}

/// An independent restatement of the security-critical invariants.
///
/// It is written from the contracts rather than from `require_profile`, so a regression that
/// loosened the real check would show up here as an accepted manifest this predicate rejects.
fn independently_compatible(manifest: &GenerationManifest) -> bool {
    const MIB: u64 = 1024 * 1024;
    let shape = manifest.shape;
    let template = &manifest.template;
    shape.memory_bytes >= 128 * MIB
        && shape.memory_bytes <= 3 * 1024 * MIB
        && shape.memory_bytes.is_multiple_of(4096)
        && shape.vcpu_count == 1
        && shape.memory_slot_layout_version == contracts::MEMORY_SLOT_LAYOUT_VERSION
        && shape.launch_page_layout_version == contracts::LAUNCH_PAGE_LAYOUT_VERSION
        && manifest.compiler_policy_version == 1
        && manifest.initramfs.layout_version == 3
        && manifest.repair.policy_version == contracts::REPAIR_POLICY_VERSION
        && manifest.tree.size > 0
        && manifest.tree.size <= 512 * MIB
        && template.ttl_seconds > 0
        && template.ttl_seconds <= 30 * 24 * 3600
        && manifest
            .overlay
            .templates
            .iter()
            .any(|entry| entry.capacity == template.writable_storage_bytes)
        && manifest
            .overlay
            .templates
            .iter()
            .all(|entry| entry.capacity >= 64 * MIB && entry.capacity.is_multiple_of(4 * MIB))
        && manifest.overlay.minimum_capacity <= manifest.overlay.maximum_capacity
        && template
            .workload_probe
            .as_ref()
            .is_none_or(|probe| probe.starts_with(b"/") && !probe.iter().any(u8::is_ascii_control))
}

#[test]
fn no_single_bit_mutation_panics_or_bypasses_validation() {
    let bytes = encode_candidate(&fixture::profile_v1()).expect("canonical bytes");
    let profile = profile();
    let mut accepted = 0_usize;

    for index in 0..bytes.len() {
        for bit in 0..8_u32 {
            let mut mutated = bytes.clone();
            mutated[index] ^= 1 << bit;
            let Ok(manifest) = decode_candidate(&mutated) else {
                continue;
            };
            // A mutation that decodes must re-encode to exactly the bytes it came from, so no
            // ambiguous, padded, or duplicated encoding can hide behind a valid manifest.
            assert_eq!(
                encode_candidate(&manifest).expect("re-encode"),
                mutated,
                "byte {index} bit {bit} decoded to a non-canonical manifest"
            );
            if require_profile(&manifest, &profile).is_err() {
                continue;
            }
            accepted += 1;
            assert!(
                independently_compatible(&manifest),
                "byte {index} bit {bit} passed compatibility while violating an invariant"
            );
        }
    }
    // Content-addressed digests and sizes, the free-form provenance string, and other valid
    // values inside their declared ranges are expected to survive; each still changes the
    // manifest identity, and the store re-verifies every object the digests name.
    assert!(
        accepted > 0,
        "the mutation sweep exercised no accepted case"
    );
}

#[test]
fn trailing_and_leading_bytes_never_decode() {
    let bytes = encode_candidate(&fixture::profile_v1()).expect("canonical bytes");

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_candidate(&trailing).is_err());

    let mut leading = vec![0_u8];
    leading.extend_from_slice(&bytes);
    assert!(decode_candidate(&leading).is_err());

    assert!(decode_candidate(&[]).is_err());
}

#[test]
fn the_launch_page_layout_version_matches_the_guest_protocol_schema() {
    assert_eq!(
        crate::generation::contracts::LAUNCH_PAGE_LAYOUT_VERSION,
        3,
        "the manifest and the launch-page schema must change together"
    );
}
