//! The bounds the OCI layout reader enforces on every blob it reads, and what it does when a
//! compressed layer is not what it claims to be.
//!
//! Each limit is set to one byte below what the layout needs, so the phase named in the failure
//! identifies exactly which read the limit stopped. The conformance of the same reader with the
//! image specification lives in `conformance.rs`.

mod support;

use std::fs;

use soma::OciPlatform;
use soma_generation::{
    ImportError, ImportErrorKind, ImportLimits, ImportOciLayout, ImportPhase, OciSelection,
    import_oci_layout,
};
use support::{Fixture, GZIP};

#[test]
fn request_blob_limit_applies_to_layout_marker() {
    let fixture = Fixture::new();
    let limits = ImportLimits {
        max_blob_bytes: 1,
        ..ImportLimits::default()
    };

    let error = run(&fixture, limits).unwrap_err();

    assert_eq!(error.phase(), ImportPhase::OpenLayout);
    assert_eq!(error.kind(), ImportErrorKind::LimitExceeded);
}

#[test]
fn request_blob_limit_applies_to_top_index() {
    let fixture = Fixture::new();
    fixture.write_index(&[]);
    let marker_size = fs::metadata(fixture.layout.join("oci-layout"))
        .unwrap()
        .len();
    let limits = ImportLimits {
        max_blob_bytes: marker_size,
        ..ImportLimits::default()
    };

    let error = run(&fixture, limits).unwrap_err();

    assert_eq!(error.phase(), ImportPhase::SelectManifest);
    assert_eq!(error.kind(), ImportErrorKind::LimitExceeded);
}

#[test]
fn compressed_layer_obeys_the_per_blob_limit() {
    let fixture = Fixture::new();
    let compressed = vec![0_u8; 1_024];
    let image = fixture.add_image(&compressed, b"expanded", GZIP);
    fixture.write_direct_index(&image, true);
    let limits = ImportLimits {
        max_blob_bytes: 512,
        ..ImportLimits::default()
    };

    let error = run(&fixture, limits).unwrap_err();

    assert_eq!(error.phase(), ImportPhase::VerifyLayer);
    assert_eq!(error.kind(), ImportErrorKind::LimitExceeded);
}

#[test]
fn malformed_gzip_layer_fails_closed() {
    let fixture = Fixture::new();
    let image = fixture.add_image(b"not a gzip stream", b"expanded", GZIP);
    fixture.write_direct_index(&image, true);

    let error = run(&fixture, ImportLimits::default()).unwrap_err();

    assert_eq!(error.phase(), ImportPhase::VerifyLayer);
    assert_eq!(error.kind(), ImportErrorKind::Io);
}

fn run(
    fixture: &Fixture,
    limits: ImportLimits,
) -> Result<soma_generation::ImportedOci, ImportError> {
    import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        limits,
    ))
}
