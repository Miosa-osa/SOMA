use std::{env, path::Path, time::Instant};

use soma::OciPlatform;
use soma_generation::{
    ImportLimits, ImportOciLayout, ImportedOci, OciSelection, import_oci_layout,
};

#[test]
#[ignore = "requires SOMA_OCI_LAYOUT from an extracted Apple Container OCI archive"]
fn imports_an_extracted_real_apple_container_layout() {
    let layout = env::var_os("SOMA_OCI_LAYOUT").expect("SOMA_OCI_LAYOUT is required");
    let temporary = tempfile::tempdir().unwrap();
    let platform = OciPlatform::linux_arm64();

    let first = timed_import(
        Path::new(&layout),
        &temporary.path().join("first"),
        &platform,
    );

    assert_eq!(first.workload().platform().operating_system(), "linux");
    assert_eq!(first.workload().platform().architecture(), "arm64");
    assert!(first.stored_blob_count() >= 6);
    assert_eq!(first.traversed_indexes().len(), 2);

    if let Some(second_layout) = env::var_os("SOMA_OCI_LAYOUT_SECOND") {
        let second = timed_import(
            Path::new(&second_layout),
            &temporary.path().join("second"),
            &platform,
        );
        assert_eq!(first.workload(), second.workload());
        assert_eq!(
            first.import_manifest_digest(),
            second.import_manifest_digest()
        );
        assert_eq!(first.import_manifest_size(), second.import_manifest_size());
        assert_eq!(first.stored_blob_count(), second.stored_blob_count());
        assert_eq!(first.stored_bytes(), second.stored_bytes());
        assert_ne!(first.traversed_indexes(), second.traversed_indexes());
    }
}

fn timed_import(layout: &Path, store: &Path, platform: &OciPlatform) -> ImportedOci {
    std::fs::create_dir(store).unwrap();
    let started = Instant::now();
    let imported = import_oci_layout(ImportOciLayout::new(
        layout,
        store,
        OciSelection::Platform(platform),
        ImportLimits::default(),
    ))
    .unwrap();
    eprintln!("imported={imported:?} elapsed={:?}", started.elapsed());
    imported
}
