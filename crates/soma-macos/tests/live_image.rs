#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::path::PathBuf;

use soma_macos::{ControlLimits, ImageBinding, ImageReference, ImageSourceReference, MacOsBackend};

#[test]
#[ignore = "requires the pinned Apple container runtime and a cached or reachable node:22 image"]
fn resolves_cached_node_22_for_linux_arm64() {
    let home = std::env::var_os("HOME").expect("macOS home directory");
    let runtime = PathBuf::from(home)
        .join("Library/Application Support/SOMA/apple-container/1.3.0/bin/container");
    let backend = MacOsBackend::with_executable(runtime);
    backend.probe().expect("pinned runtime is ready");
    let image = ImageReference::new("node:22").expect("image reference");
    let limits = ControlLimits::new(30_000, 1_048_576).expect("bounded image resolution");

    let resolved = backend
        .resolve_image(&image, limits)
        .expect("resolve node:22");

    assert_eq!(resolved.platform().os(), "linux");
    assert_eq!(resolved.platform().architecture(), "arm64");
    assert_eq!(resolved.platform().variant(), Some("v8"));
    assert_eq!(resolved.source_reference(), ImageSourceReference::Redacted);
    assert_eq!(resolved.binding(), ImageBinding::ObservedOnlyNotEnforced);
    assert!(resolved.index_digest().as_str().starts_with("sha256:"));
    assert!(resolved.manifest_digest().as_str().starts_with("sha256:"));
    eprintln!(
        "index={} manifest={} pull_ms={} inspect_ms={}",
        resolved.index_digest().as_str(),
        resolved.manifest_digest().as_str(),
        resolved.timings().pull_millis(),
        resolved.timings().inspect_millis(),
    );
}
