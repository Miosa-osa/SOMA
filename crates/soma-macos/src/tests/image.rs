use crate::{
    BackendError, ExecutionStatus, ImageBinding, ImageReference, ImageResolutionFailure,
    ImageSourceReference,
};

use super::fixtures::{backend, control_limits, output_at, strings};

const INSPECT_NODE_22: &[u8] = include_bytes!("data/apple-container-1.3-node-22-inspect.json");

#[test]
fn resolve_image_returns_exact_arm64_manifest_and_index_identity_with_separate_timings() {
    let (backend, runner) = backend([
        Ok(output_at(
            ExecutionStatus::Exited { code: 0 },
            Vec::new(),
            Vec::new(),
            431,
        )),
        Ok(output_at(
            ExecutionStatus::Exited { code: 0 },
            INSPECT_NODE_22,
            Vec::new(),
            19,
        )),
    ]);
    let image = ImageReference::new("node:22").expect("image");

    let resolved = backend
        .resolve_image(&image, control_limits())
        .expect("resolve cached image");

    assert_eq!(
        resolved.index_digest().as_str(),
        "sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d"
    );
    assert_eq!(
        resolved.manifest_digest().as_str(),
        "sha256:2f22d3b5ec6552b890773a152030b1360d35da0c4369799319523ccdb2d78e0e"
    );
    assert_eq!(resolved.platform().os(), "linux");
    assert_eq!(resolved.platform().architecture(), "arm64");
    assert_eq!(resolved.platform().variant(), Some("v8"));
    assert_eq!(resolved.source_reference(), ImageSourceReference::Redacted);
    assert_eq!(resolved.binding(), ImageBinding::ObservedOnlyNotEnforced);
    assert_eq!(resolved.timings().pull_millis(), 431);
    assert_eq!(resolved.timings().inspect_millis(), 19);

    let receipt = serde_json::to_value(&resolved).expect("resolution receipt");
    assert_eq!(receipt["source_reference"], "redacted");
    assert_eq!(receipt["binding"], "observed_only_not_enforced");

    let calls = runner.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].arguments,
        strings(&[
            "image",
            "pull",
            "--progress",
            "none",
            "--platform",
            "linux/arm64",
            "node:22",
        ])
    );
    assert_eq!(
        calls[1].arguments,
        strings(&["image", "inspect", "node:22"])
    );
}

#[test]
fn resolve_image_fails_closed_for_missing_multiple_or_mismatched_variants() {
    assert_resolution_failure(
        &record("[]", INDEX_DIGEST, INDEX_HEX),
        ImageResolutionFailure::MissingVariant,
    );
    let two_variants = format!(
        "[{},{}]",
        variant("linux", "arm64", MANIFEST_DIGEST),
        variant("linux", "arm64", MANIFEST_DIGEST)
    );
    assert_resolution_failure(
        &record(&two_variants, INDEX_DIGEST, INDEX_HEX),
        ImageResolutionFailure::MultipleVariants,
    );
    assert_resolution_failure(
        &record(
            &format!("[{}]", variant("linux", "amd64", MANIFEST_DIGEST)),
            INDEX_DIGEST,
            INDEX_HEX,
        ),
        ImageResolutionFailure::PlatformMismatch,
    );
    assert_resolution_failure(
        &record(
            &format!(
                "[{}]",
                variant_with_variant("linux", "arm64", "v7", MANIFEST_DIGEST)
            ),
            INDEX_DIGEST,
            INDEX_HEX,
        ),
        ImageResolutionFailure::PlatformMismatch,
    );
}

#[test]
fn resolve_image_rejects_ambiguous_records_and_malformed_or_inconsistent_digests() {
    assert_resolution_failure("[]", ImageResolutionFailure::MissingImageRecord);
    let valid = record(
        &format!("[{}]", variant("linux", "arm64", MANIFEST_DIGEST)),
        INDEX_DIGEST,
        INDEX_HEX,
    );
    let valid_record = valid
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .expect("record wrapper");
    assert_resolution_failure(
        &format!("[{valid_record},{valid_record}]"),
        ImageResolutionFailure::MultipleImageRecords,
    );
    assert_resolution_failure(
        &record(
            &format!("[{}]", variant("linux", "arm64", MANIFEST_DIGEST)),
            "sha256:not-a-digest",
            INDEX_HEX,
        ),
        ImageResolutionFailure::MalformedIndexDigest,
    );
    assert_resolution_failure(
        &record(
            &format!("[{}]", variant("linux", "arm64", "sha256:bad")),
            INDEX_DIGEST,
            INDEX_HEX,
        ),
        ImageResolutionFailure::MalformedManifestDigest,
    );
    assert_resolution_failure(
        &record(
            &format!("[{}]", variant("linux", "arm64", MANIFEST_DIGEST)),
            INDEX_DIGEST,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        ImageResolutionFailure::IndexIdentityMismatch,
    );
}

const INDEX_DIGEST: &str =
    "sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d";
const INDEX_HEX: &str = "8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d";
const MANIFEST_DIGEST: &str =
    "sha256:2f22d3b5ec6552b890773a152030b1360d35da0c4369799319523ccdb2d78e0e";

fn assert_resolution_failure(document: &str, expected: ImageResolutionFailure) {
    let (backend, _) = backend([
        Ok(output_at(
            ExecutionStatus::Exited { code: 0 },
            Vec::new(),
            Vec::new(),
            1,
        )),
        Ok(output_at(
            ExecutionStatus::Exited { code: 0 },
            document.as_bytes(),
            Vec::new(),
            1,
        )),
    ]);
    let image = ImageReference::new("node:22").expect("image");

    assert_eq!(
        backend
            .resolve_image(&image, control_limits())
            .expect_err("invalid image metadata must fail closed"),
        BackendError::ImageResolution { failure: expected }
    );
}

fn record(variants: &str, index_digest: &str, id: &str) -> String {
    format!(
        r#"[{{"configuration":{{"descriptor":{{"digest":"{index_digest}"}}}},"id":"{id}","variants":{variants}}}]"#
    )
}

fn variant(os: &str, architecture: &str, digest: &str) -> String {
    variant_with_variant(os, architecture, "v8", digest)
}

fn variant_with_variant(os: &str, architecture: &str, variant: &str, digest: &str) -> String {
    format!(
        r#"{{"digest":"{digest}","platform":{{"architecture":"{architecture}","os":"{os}","variant":"{variant}"}}}}"#
    )
}
