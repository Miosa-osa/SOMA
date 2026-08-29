//! P0.1 gate: a reusable Generation carries public identity only.
//!
//! Every byte a party can retrieve from the content store is scanned for the four secret
//! fields of one Instance launch page, including the responder static secret that layout v2
//! used to bake into the initramfs.

mod support;

use std::{convert::Infallible, fs, path::Path};

use soma_generation::{
    CompiledCandidate, SnapshotBinding, generation_manifest::encode_manifest,
    initramfs::INITRAMFS_LAYOUT_VERSION,
};
use soma_guest::{HostLaunchMaterial, LAUNCH_PAGE_SIZE, LaunchNetwork};
use support::{
    fixture_tree::{AGENT, fixture_layers},
    generation::{compile, toolchains},
    rootfs::normalize_layers_for,
};

/// The launch-page byte ranges that must never appear in a retrievable artifact.
const SECRET_FIELDS: [(&str, usize, usize); 4] = [
    ("launch nonce", 84, 116),
    ("Instance PSK", 116, 148),
    ("entropy seed", 148, 212),
    ("responder static secret", 247, 279),
];

fn launch_page(generation: [u8; 32], instance: [u8; 16]) -> [u8; LAUNCH_PAGE_SIZE] {
    let network = LaunchNetwork::new(
        3,
        1,
        [0x02, 0, 0, 0, 0, 1],
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        1,
    )
    .expect("fixed test network");
    let host = HostLaunchMaterial::generate(generation, instance, [3; 16], network)
        .expect("fresh Instance authority");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    host.deliver_with(|bytes| {
        page.copy_from_slice(bytes);
        Ok::<(), Infallible>(())
    })
    .expect("page delivery");
    page
}

fn generation_bytes(compiled: &CompiledCandidate) -> [u8; 32] {
    let hex = compiled
        .id()
        .as_str()
        .strip_prefix("sha256:")
        .expect("GenerationId prefix");
    let mut bytes = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks(2).enumerate() {
        bytes[index] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    bytes
}

/// Every published object in the store, named by its file name.
fn published_objects(store: &Path) -> Vec<(String, Vec<u8>)> {
    let blobs = store.join("v1/blobs/sha256");
    let mut objects: Vec<(String, Vec<u8>)> = fs::read_dir(&blobs)
        .expect("published blob directory")
        .map(|entry| {
            let entry = entry.expect("blob entry");
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).expect("blob bytes"),
            )
        })
        .collect();
    objects.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        objects.len() >= 6,
        "expected the kernel, initramfs, agent, root, templates, and manifest"
    );
    objects
}

#[test]
fn no_published_generation_artifact_contains_instance_launch_secrets() {
    let Some(tools) =
        toolchains("no_published_generation_artifact_contains_instance_launch_secrets")
    else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT)
        .expect("compiled Generation");
    let manifest = &compiled.candidate.manifest;
    assert_eq!(manifest.snapshot, SnapshotBinding::Absent);
    assert_eq!(manifest.initramfs.layout_version, INITRAMFS_LAYOUT_VERSION);

    let generation = generation_bytes(&compiled);
    let pages = [
        launch_page(generation, [0x11; 16]),
        launch_page(generation, [0x22; 16]),
    ];
    let named = [
        ("kernel", &manifest.kernel.descriptor),
        ("initramfs", &manifest.initramfs.descriptor),
        ("guest agent", &manifest.guest_agent.descriptor),
        ("EROFS root", &manifest.root.descriptor),
        (
            "overlay template",
            &manifest.overlay.templates[0].descriptor,
        ),
    ];
    let mut objects = published_objects(&fixture.store);
    objects.push((
        "encoded manifest".to_owned(),
        encode_manifest(manifest).expect("manifest bytes"),
    ));
    for page in &pages {
        for (field, start, end) in SECRET_FIELDS {
            let secret = &page[start..end];
            assert!(secret.iter().any(|byte| *byte != 0), "{field} was zero");
            for (name, bytes) in &objects {
                assert!(
                    !bytes.windows(secret.len()).any(|window| window == secret),
                    "{field} appears in published object {name}"
                );
            }
        }
    }
    for (role, descriptor) in named {
        assert!(
            objects.iter().any(
                |(name, _)| descriptor.digest.to_string().strip_prefix("sha256:")
                    == Some(name.as_str())
            ),
            "the {role} object is missing from the store"
        );
    }
    for retired in [b"etc/soma/responder.key".as_slice(), b"etc/soma".as_slice()] {
        for (name, bytes) in &objects {
            assert!(
                !bytes.windows(retired.len()).any(|window| window == retired),
                "published object {name} still carries {}",
                String::from_utf8_lossy(retired)
            );
        }
    }
}
