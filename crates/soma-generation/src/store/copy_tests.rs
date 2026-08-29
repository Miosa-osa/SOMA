use std::io::Cursor;

use crate::{
    ImportErrorKind, ImportPhase, digest,
    oci::{Descriptor, PLAIN_LAYER},
};

#[test]
fn growing_source_reads_only_expected_bytes_plus_eof_probe() {
    let (_temporary, root, store) = test_store();
    let mut source = Cursor::new(vec![0x5a; 64 * 1024]);
    let descriptor = descriptor(&[0x5a; 2]);

    let error = store
        .stage_descriptor(
            &mut source,
            &descriptor,
            descriptor.size,
            ImportPhase::VerifyLayer,
        )
        .err()
        .expect("a growing descriptor source must fail");

    assert_eq!(error.kind(), ImportErrorKind::Integrity);
    assert_eq!(source.position(), 3);
    assert_eq!(std::fs::read_dir(root.join("v1/tmp")).unwrap().count(), 0);
}

#[test]
fn truncated_source_is_rejected_during_copy() {
    let (_temporary, root, store) = test_store();
    let mut source = Cursor::new([0x5a]);
    let descriptor = descriptor(&[0x5a; 2]);

    let error = store
        .stage_descriptor(
            &mut source,
            &descriptor,
            descriptor.size,
            ImportPhase::VerifyLayer,
        )
        .err()
        .expect("a truncated descriptor source must fail");

    assert_eq!(error.kind(), ImportErrorKind::Integrity);
    assert_eq!(std::fs::read_dir(root.join("v1/tmp")).unwrap().count(), 0);
}

#[test]
fn substituted_staged_path_is_rejected_and_removed() {
    let (_temporary, root, store) = test_store();
    let expected = b"verified";
    let descriptor = descriptor(expected);
    let staged = store
        .stage_descriptor(
            &mut Cursor::new(expected),
            &descriptor,
            descriptor.size,
            ImportPhase::VerifyLayer,
        )
        .unwrap();
    let staging_root = root.join("v1/tmp");
    let staged_path = std::fs::read_dir(&staging_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();

    // The completed stage is the deterministic barrier before name-based linking.
    std::fs::remove_file(&staged_path).unwrap();
    std::fs::write(&staged_path, b"attacker").unwrap();
    let error = staged.publish().unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::StoreConflict);
    assert!(
        !root
            .join("v1/blobs/sha256")
            .join(&descriptor.digest.as_str()[7..])
            .exists()
    );
    assert_eq!(std::fs::read_dir(staging_root).unwrap().count(), 0);
}

fn test_store() -> (tempfile::TempDir, std::path::PathBuf, super::Store) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("store");
    std::fs::create_dir(&root).unwrap();
    let store = super::Store::open(&root).unwrap();
    (temporary, root, store)
}

fn descriptor(bytes: &[u8]) -> Descriptor {
    Descriptor {
        media_type: PLAIN_LAYER.to_owned(),
        digest: digest::bytes(bytes),
        size: u64::try_from(bytes.len()).unwrap(),
        platform: None,
    }
}
