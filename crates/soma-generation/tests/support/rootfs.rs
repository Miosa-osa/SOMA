use serde_json::json;
use soma::OciPlatform;
use soma_generation::{
    ImportLimits, ImportOciLayout, NormalizeOciRootfs, NormalizedRootfs, OciSelection,
    RootfsLimits, import_oci_layout, normalize_oci_rootfs,
};
use tar::EntryType;

use super::{CONFIG, Fixture, Image, MANIFEST, PLAIN, descriptor, digest};

mod tree_manifest;

pub fn read_tree(
    store: &std::path::Path,
    digest: &soma::OciDigest,
) -> Vec<tree_manifest::TreeEntry> {
    tree_manifest::read_tree(store, digest)
}

pub struct TarEntry<'a> {
    path: &'a [u8],
    kind: EntryType,
    body: &'a [u8],
    link: Option<&'a [u8]>,
    mode: u32,
    uid: u64,
    gid: u64,
    mtime: u64,
}

impl<'a> TarEntry<'a> {
    pub fn file(path: &'a [u8], body: &'a [u8]) -> Self {
        Self::new(path, EntryType::Regular, body)
    }

    pub fn directory(path: &'a [u8]) -> Self {
        Self::new(path, EntryType::Directory, &[])
    }

    pub fn symlink(path: &'a [u8], target: &'a [u8]) -> Self {
        Self::new(path, EntryType::Symlink, &[]).link(target)
    }

    pub fn hardlink(path: &'a [u8], target: &'a [u8]) -> Self {
        Self::new(path, EntryType::Link, &[]).link(target)
    }

    pub fn fifo(path: &'a [u8]) -> Self {
        Self::new(path, EntryType::Fifo, &[])
    }

    pub fn special(path: &'a [u8], kind: EntryType) -> Self {
        Self::new(path, kind, &[])
    }

    pub fn mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self
    }

    pub fn ownership(mut self, uid: u64, gid: u64) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    pub fn mtime(mut self, mtime: u64) -> Self {
        self.mtime = mtime;
        self
    }

    fn new(path: &'a [u8], kind: EntryType, body: &'a [u8]) -> Self {
        Self {
            path,
            kind,
            body,
            link: None,
            mode: if kind.is_dir() { 0o755 } else { 0o644 },
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    fn link(mut self, target: &'a [u8]) -> Self {
        self.link = Some(target);
        self
    }
}

pub fn tar(entries: &[TarEntry<'_>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        for entry in entries {
            append_entry(&mut builder, entry);
        }
        builder.finish().unwrap();
    }
    bytes
}

pub fn add_layers(fixture: &Fixture, layers: &[Vec<u8>]) -> Image {
    add_layers_for(fixture, layers, "arm64")
}

pub fn add_layers_for(fixture: &Fixture, layers: &[Vec<u8>], architecture: &str) -> Image {
    let layer_descriptors: Vec<_> = layers
        .iter()
        .map(|layer| {
            let layer_digest = fixture.put_blob(layer);
            descriptor(PLAIN, &layer_digest, layer.len())
        })
        .collect();
    let config = serde_json::to_vec(&json!({
        "architecture": architecture,
        "os": "linux",
        "rootfs": {
            "type": "layers",
            "diff_ids": layers.iter().map(|layer| digest(layer)).collect::<Vec<_>>(),
        },
    }))
    .unwrap();
    let config_digest = fixture.put_blob(&config);
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST,
        "config": descriptor(CONFIG, &config_digest, config.len()),
        "layers": layer_descriptors,
    }))
    .unwrap();
    let manifest_digest = fixture.put_blob(&manifest);
    Image {
        manifest_digest,
        manifest_size: manifest.len(),
        config_digest,
        layer_digest: layers
            .first()
            .map_or_else(String::new, |layer| digest(layer)),
    }
}

pub fn normalize_layers(layers: &[Vec<u8>]) -> (Fixture, NormalizedRootfs) {
    normalize_layers_for(layers, "arm64")
}

pub fn normalize_layers_for(layers: &[Vec<u8>], architecture: &str) -> (Fixture, NormalizedRootfs) {
    let fixture = Fixture::new();
    let image = add_layers_for(&fixture, layers, architecture);
    fixture.write_direct_index(&image, architecture == "arm64");
    let platform = if architecture == "amd64" {
        OciPlatform::linux_amd64()
    } else {
        OciPlatform::linux_arm64()
    };
    let normalized = normalize_existing(&fixture, &platform);
    (fixture, normalized)
}

/// Imports and normalizes the image an already written layout holds for one platform.
pub fn normalize_existing(fixture: &Fixture, platform: &OciPlatform) -> NormalizedRootfs {
    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(platform),
        ImportLimits::default(),
    ))
    .unwrap();
    normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &fixture.store,
        RootfsLimits::default(),
    ))
    .unwrap()
}

pub fn pax_layer(path: &str, key: &str, value: &[u8]) -> Vec<u8> {
    local_pax_layer(&TarEntry::file(path.as_bytes(), b"pax"), &[(key, value)])
}

pub fn local_pax_layer(entry: &TarEntry<'_>, extensions: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        builder
            .append_pax_extensions(extensions.iter().copied())
            .unwrap();
        append_entry(&mut builder, entry);
        builder.finish().unwrap();
    }
    bytes
}

pub fn global_pax_layer() -> Vec<u8> {
    let record = b"17 comment=value\n";
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let mut extension = tar::Header::new_ustar();
        extension.set_path("GlobalHead").unwrap();
        extension.set_entry_type(EntryType::XGlobalHeader);
        extension.set_size(record.len() as u64);
        extension.set_mode(0o644);
        extension.set_cksum();
        builder.append(&extension, record.as_slice()).unwrap();
        let entry = TarEntry::file(b"value", b"global-pax");
        append_entry(&mut builder, &entry);
        builder.finish().unwrap();
    }
    bytes
}

pub fn malformed_local_pax_layer(record: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let mut extension = tar::Header::new_ustar();
        extension.set_path("PaxHeader").unwrap();
        extension.set_entry_type(EntryType::XHeader);
        extension.set_size(record.len() as u64);
        extension.set_mode(0o644);
        extension.set_cksum();
        builder.append(&extension, record).unwrap();
        append_entry(&mut builder, &TarEntry::file(b"value", b"malformed-pax"));
        builder.finish().unwrap();
    }
    bytes
}

pub fn sparse_layer() -> Vec<u8> {
    let body = [b'x'; 512];
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let mut header = tar::Header::new_gnu();
        set_raw(&mut header.as_old_mut().name, b"sparse");
        header.set_entry_type(EntryType::GNUSparse);
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        let gnu = header.as_gnu_mut().unwrap();
        gnu.set_real_size(1024);
        gnu.sparse[0].set_offset(512);
        gnu.sparse[0].set_length(512);
        header.set_cksum();
        builder.append(&header, body.as_slice()).unwrap();
        builder.finish().unwrap();
    }
    bytes
}

fn set_raw(field: &mut [u8], value: &[u8]) {
    assert!(value.len() < field.len());
    field.fill(0);
    field[..value.len()].copy_from_slice(value);
}

fn append_entry(builder: &mut tar::Builder<&mut Vec<u8>>, entry: &TarEntry<'_>) {
    let mut header = tar::Header::new_old();
    set_raw(&mut header.as_old_mut().name, entry.path);
    if let Some(link) = entry.link {
        set_raw(&mut header.as_old_mut().linkname, link);
    }
    header.set_entry_type(entry.kind);
    header.set_size(entry.body.len() as u64);
    header.set_mode(entry.mode);
    header.set_uid(entry.uid);
    header.set_gid(entry.gid);
    header.set_mtime(entry.mtime);
    header.set_cksum();
    builder.append(&header, entry.body).unwrap();
}
