//! Tests for admission: what it retains, and what it refuses to admit.
//!
//! Every case is written against a real published store so that the refusals come from the same
//! code path a host runs, not from a stub that agrees with the test.

use std::{fs, io::Cursor};

use super::*;
use crate::{
    digest,
    generation::{
        artifacts::{ArtifactRole, Sha256Digest},
        identity::derive_generation_id,
        manifest::{encode_manifest, fixture},
    },
};

const BLOCK: u64 = 4096;
const OVERLAY: u64 = 64 * 1024 * 1024;

fn resize(descriptor: &mut ArtifactDescriptor, size: u64) {
    let fill = descriptor.role.code();
    let bytes = vec![fill; usize::try_from(size).unwrap()];
    descriptor.size = size;
    descriptor.digest = Sha256Digest::from_oci(&digest::bytes(&bytes));
}

fn installed() -> (
    tempfile::TempDir,
    GenerationId,
    CompilerProfile,
    Vec<ArtifactDescriptor>,
) {
    let root = tempfile::tempdir().expect("store root");
    let store = Store::open(root.path()).expect("open store");
    let mut manifest = fixture::profile_v1();
    manifest.overlay.templates.truncate(1);
    manifest.overlay.templates[0].capacity = OVERLAY;
    manifest.overlay.minimum_capacity = OVERLAY;
    manifest.overlay.maximum_capacity = OVERLAY;
    manifest.template.writable_storage_bytes = OVERLAY;
    manifest.snapshot = fixture::captured_snapshot();
    resize(&mut manifest.kernel.descriptor, BLOCK);
    resize(&mut manifest.initramfs.descriptor, BLOCK);
    resize(&mut manifest.root.descriptor, BLOCK);
    resize(&mut manifest.overlay.templates[0].descriptor, OVERLAY);
    if let SnapshotBinding::Captured {
        memory,
        overlay,
        state,
        ..
    } = &mut manifest.snapshot
    {
        resize(memory, BLOCK);
        resize(overlay, BLOCK);
        resize(state, BLOCK);
    }
    let descriptors = launch_descriptors(&manifest);
    for descriptor in &descriptors {
        let bytes = vec![descriptor.role.code(); usize::try_from(descriptor.size).unwrap()];
        store
            .put_descriptor(
                &mut Cursor::new(bytes),
                &descriptor.to_store_descriptor(),
                descriptor.size,
                ImportPhase::Publish,
            )
            .expect("publish artifact");
    }
    let bytes = encode_manifest(&manifest).expect("encode ready manifest");
    store
        .put_bytes(
            &bytes,
            ArtifactRole::GenerationManifest.media_type(),
            ImportPhase::Publish,
        )
        .expect("publish ready manifest");
    let mut profile = CompilerProfile::v1();
    profile.overlay_capacities = vec![OVERLAY];
    (root, derive_generation_id(&bytes), profile, descriptors)
}

#[test]
fn admission_retains_every_digest_verified_launch_handle() {
    let (root, id, profile, descriptors) = installed();
    let admitted = admit_installed_generation(root.path(), &id, &profile).expect("admit");

    assert_eq!(admitted.artifacts.len(), descriptors.len());
}

#[test]
fn same_size_corruption_is_refused() {
    let (root, id, profile, descriptors) = installed();
    let target = &descriptors[0];
    let path = root
        .path()
        .join("v1/blobs/sha256")
        .join(crate::digest::hex(&target.digest.to_oci()));
    fs::remove_file(&path).expect("remove verified artifact");
    fs::write(&path, vec![0xff; usize::try_from(target.size).unwrap()])
        .expect("replace with same-size corruption");

    assert!(admit_installed_generation(root.path(), &id, &profile).is_err());
}

#[cfg(unix)]
#[test]
fn a_symlinked_artifact_is_refused() {
    let (root, id, profile, descriptors) = installed();
    let target = &descriptors[0];
    let path = root
        .path()
        .join("v1/blobs/sha256")
        .join(crate::digest::hex(&target.digest.to_oci()));
    let substitute = root.path().join("same-size-substitute");
    fs::write(
        &substitute,
        vec![0_u8; usize::try_from(target.size).unwrap()],
    )
    .expect("write substitute");
    fs::remove_file(&path).expect("remove artifact");
    std::os::unix::fs::symlink(&substitute, &path).expect("replace with symlink");

    assert!(admit_installed_generation(root.path(), &id, &profile).is_err());
}

#[test]
fn a_manifest_without_a_snapshot_is_never_admitted() {
    let root = tempfile::tempdir().expect("store root");
    let store = Store::open(root.path()).expect("open store");
    let manifest = fixture::profile_v1();
    let bytes = encode_manifest(&manifest).expect("encode manifest");
    store
        .put_bytes(
            &bytes,
            ArtifactRole::GenerationManifest.media_type(),
            ImportPhase::Publish,
        )
        .expect("publish manifest");

    let id = derive_generation_id(&bytes);
    assert!(admit_installed_generation(root.path(), &id, &CompilerProfile::v1()).is_err());
}
