#![allow(dead_code)]

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub const INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const CONFIG: &str = "application/vnd.oci.image.config.v1+json";
pub const PLAIN: &str = "application/vnd.oci.image.layer.v1.tar";
pub const GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

pub struct Fixture {
    temporary: tempfile::TempDir,
    pub layout: PathBuf,
    pub store: PathBuf,
}

pub struct Image {
    pub manifest_digest: String,
    pub manifest_size: usize,
    pub config_digest: String,
    pub layer_digest: String,
}

impl Fixture {
    pub fn new() -> Self {
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
        Self {
            temporary,
            layout,
            store,
        }
    }

    pub fn add_image(
        &self,
        layer_blob: &[u8],
        expanded_layer: &[u8],
        layer_media_type: &str,
    ) -> Image {
        let layer_digest = self.put_blob(layer_blob);
        let config = serde_json::to_vec(&json!({
            "architecture": "arm64",
            "os": "linux",
            "rootfs": {"type": "layers", "diff_ids": [digest(expanded_layer)]},
        }))
        .unwrap();
        let config_digest = self.put_blob(&config);
        let manifest = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST,
            "config": descriptor(CONFIG, &config_digest, config.len()),
            "layers": [descriptor(layer_media_type, &layer_digest, layer_blob.len())],
        }))
        .unwrap();
        let manifest_digest = self.put_blob(&manifest);
        Image {
            manifest_digest,
            manifest_size: manifest.len(),
            config_digest,
            layer_digest,
        }
    }

    pub fn add_plain_image(&self, contents: &[u8]) -> Image {
        let layer = tar_layer(contents);
        self.add_image(&layer, &layer, PLAIN)
    }

    pub fn write_direct_index(&self, image: &Image, platform: bool) {
        let mut descriptor = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
        if platform {
            descriptor["platform"] = json!({"os": "linux", "architecture": "arm64"});
        }
        self.write_index(&[descriptor]);
    }

    pub fn write_index(&self, descriptors: &[Value]) {
        fs::write(
            self.layout.join("index.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "mediaType": INDEX,
                "manifests": descriptors,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    pub fn write_nested_index(&self, image: &Image, annotation_order: bool) -> String {
        let annotations = if annotation_order {
            r#""annotations":{"alpha":"1","beta":"2"},"#
        } else {
            r#""annotations":{"beta":"2","alpha":"1"},"#
        };
        let nested = format!(
            "{{\"schemaVersion\":2,\"mediaType\":\"{INDEX}\",{annotations}\"manifests\":[{{\"mediaType\":\"{MANIFEST}\",\"digest\":\"{}\",\"size\":{},\"platform\":{{\"os\":\"linux\",\"architecture\":\"arm64\"}}}}]}}",
            image.manifest_digest, image.manifest_size
        );
        let nested_digest = self.put_blob(nested.as_bytes());
        self.write_index(&[descriptor(INDEX, &nested_digest, nested.len())]);
        nested_digest
    }

    pub fn put_blob(&self, bytes: &[u8]) -> String {
        let value = digest(bytes);
        fs::write(self.layout.join("blobs/sha256").join(&value[7..]), bytes).unwrap();
        value
    }

    pub fn blob_path(&self, digest: &str) -> PathBuf {
        self.layout.join("blobs/sha256").join(&digest[7..])
    }

    pub fn keepalive(&self) -> &Path {
        self.temporary.path()
    }
}

pub fn descriptor(media_type: &str, digest: &str, size: usize) -> Value {
    json!({"mediaType": media_type, "digest": digest, "size": size})
}

pub fn digest(bytes: &[u8]) -> String {
    let mut value = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

pub fn tar_layer(contents: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let mut header = tar::Header::new_ustar();
        header.set_path("soma-fixture/payload").unwrap();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        builder.append(&header, contents).unwrap();
        builder.finish().unwrap();
    }
    bytes
}
