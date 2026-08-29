use std::{env, path::Path, time::Instant};

use soma::{OciDigest, OciPlatform};
use soma_generation::{
    ImportLimits, ImportOciLayout, NormalizeOciRootfs, OciSelection, RootfsLimits,
    import_oci_layout, normalize_oci_rootfs,
};

#[test]
#[ignore = "requires a pinned node:22 Apple Container layout and expected evidence values"]
fn normalizes_a_pinned_node_22_apple_container_rootfs() {
    let layout = env::var_os("SOMA_OCI_LAYOUT").expect("SOMA_OCI_LAYOUT is required");
    let expected_import = expected_digest("SOMA_EXPECTED_IMPORT_DIGEST");
    let expected_tree = expected_digest("SOMA_EXPECTED_ROOTFS_DIGEST");
    let expected_size = expected_u64("SOMA_EXPECTED_ROOTFS_SIZE");
    let expected_entries = u32::try_from(expected_u64("SOMA_EXPECTED_ROOTFS_ENTRIES")).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let store = temporary.path().join("store");
    std::fs::create_dir(&store).unwrap();
    let platform = OciPlatform::linux_arm64();
    let imported = import_oci_layout(ImportOciLayout::new(
        Path::new(&layout),
        &store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .unwrap();

    let started = Instant::now();
    let first = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &store,
        RootfsLimits::default(),
    ))
    .unwrap();
    let second = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &store,
        RootfsLimits::default(),
    ))
    .unwrap();

    assert_eq!(first.tree_manifest_digest(), second.tree_manifest_digest());
    assert_eq!(first.tree_manifest_size(), second.tree_manifest_size());
    assert_eq!(first.entry_count(), second.entry_count());
    eprintln!("normalized={first:?} elapsed={:?}", started.elapsed());
    assert_eq!(first.source_import_manifest_digest(), &expected_import);
    assert_eq!(first.tree_manifest_digest(), &expected_tree);
    assert_eq!(first.tree_manifest_size(), expected_size);
    assert_eq!(first.entry_count(), expected_entries);
}

fn expected_digest(name: &str) -> OciDigest {
    OciDigest::parse(env::var(name).unwrap_or_else(|_| panic!("{name} is required"))).unwrap()
}

fn expected_u64(name: &str) -> u64 {
    env::var(name)
        .unwrap_or_else(|_| panic!("{name} is required"))
        .parse()
        .unwrap()
}
