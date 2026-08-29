mod support;

use std::{fs, path::Path};

use serde_json::json;
use sha2::{Digest as _, Sha256};
use soma::{OciDigest, OciPlatform};
use soma_generation::{ImportLimits, ImportOciLayout, OciSelection, import_oci_layout};

#[test]
fn imports_one_direct_linux_amd64_manifest_into_the_content_store() {
    let temporary = tempfile::tempdir().unwrap();
    let layout = temporary.path().join("layout");
    let store = temporary.path().join("store");
    fs::create_dir_all(layout.join("blobs/sha256")).unwrap();
    fs::create_dir(&store).unwrap();
    fs::write(
        layout.join("oci-layout"),
        br#"{"imageLayoutVersion":"1.0.0"}"#,
    )
    .unwrap();

    let layer = support::tar_layer(b"plain deterministic layer");
    let layer_digest = put_blob(&layout, &layer);
    let config = serde_json::to_vec(&json!({
        "architecture": "amd64",
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [layer_digest]},
    }))
    .unwrap();
    let config_digest = put_blob(&layout, &config);
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": descriptor("application/vnd.oci.image.config.v1+json", &config_digest, config.len()),
        "layers": [descriptor("application/vnd.oci.image.layer.v1.tar", &layer_digest, layer.len())],
    }))
    .unwrap();
    let manifest_digest = put_blob(&layout, &manifest);
    fs::write(
        layout.join("index.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [{
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "digest": manifest_digest,
                "size": manifest.len(),
                "platform": {"os": "linux", "architecture": "amd64"},
            }],
        }))
        .unwrap(),
    )
    .unwrap();

    let platform = OciPlatform::linux_amd64();
    let imported = import_oci_layout(ImportOciLayout::new(
        &layout,
        &store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .unwrap();

    assert_eq!(imported.workload().platform(), &platform);
    assert_eq!(
        imported.workload().manifest_digest().as_str(),
        manifest_digest
    );
    assert_eq!(imported.stored_blob_count(), 5);
    let artifact = blob_path(&store, imported.import_manifest_digest());
    assert!(artifact.is_file());
    let import_manifest = fs::read(artifact).unwrap();
    assert_eq!(
        digest(&import_manifest),
        imported.import_manifest_digest().as_str()
    );
    let import_manifest: serde_json::Value = serde_json::from_slice(&import_manifest).unwrap();
    assert_eq!(import_manifest["layers"][0]["diff_id"], layer_digest);
    assert_eq!(import_manifest["layers"][0]["expanded_size"], layer.len());
    assert_eq!(import_manifest["layers"][0]["entry_count"], 1);
}

fn descriptor(media_type: &str, digest: &str, size: usize) -> serde_json::Value {
    json!({"mediaType": media_type, "digest": digest, "size": size})
}

fn put_blob(layout: &Path, bytes: &[u8]) -> String {
    let digest = digest(bytes);
    fs::write(layout.join("blobs/sha256").join(&digest[7..]), bytes).unwrap();
    digest
}

fn digest(bytes: &[u8]) -> String {
    let mut value = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

fn blob_path(store: &Path, digest: &OciDigest) -> std::path::PathBuf {
    store.join("v1/blobs/sha256").join(&digest.as_str()[7..])
}
