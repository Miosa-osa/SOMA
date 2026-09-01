//! Fan behaviour that does not need a reflink filesystem.

use std::num::NonZeroUsize;
use std::os::unix::fs::MetadataExt as _;

use super::*;

fn template(directory: &Path, bytes: &[u8]) -> File {
    let path = directory.join("template.ext4");
    std::fs::write(&path, bytes).expect("write template");
    File::open(&path).expect("open template")
}

fn count(directory: &Path) -> usize {
    std::fs::read_dir(directory)
        .expect("read fan")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("template").is_file())
        .count()
}

fn digest(template: &File) -> [u8; 32] {
    digest_of(template).expect("digest template")
}

#[test]
fn warms_the_requested_copies_and_reuses_them() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &vec![7_u8; 1024 * 1024]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(4).expect("nonzero");
    let first = warm(&template, &root, copies).expect("warm");
    assert_eq!(first.copies, 4);
    assert_eq!(first.written, 4);
    let second = warm(&template, &root, copies).expect("warm again");
    assert_eq!(second.written, 0);
    assert_eq!(second.key, first.key);
    assert_eq!(count(&root.join(first.key)), 4);
}

#[test]
fn every_replica_is_the_template_byte_for_byte() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let bytes: Vec<u8> = (0..(3 * 1024 * 1024_u32))
        .map(|value| u8::try_from(value % 256).unwrap_or_default())
        .collect();
    let template = template(scratch.path(), &bytes);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(3).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    for index in 0..report.copies {
        let path = root.join(&report.key).join(replica_path(index));
        assert_eq!(std::fs::read(&path).expect("read replica"), bytes);
    }
}

#[test]
fn a_replica_is_opened_only_when_the_fan_is_there() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[3_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(2).expect("nonzero");
    let identity = digest(&template);
    assert!(open_replica(&template, identity, &root, copies).is_none());
    let report = warm(&template, &root, copies).expect("warm");
    let replica = open_replica(&template, identity, &root, copies).expect("replica");
    assert_eq!(replica.metadata().expect("metadata").size(), 4096);
    assert_ne!(
        replica.metadata().expect("metadata").ino(),
        template.metadata().expect("metadata").ino()
    );
    assert_eq!(report.key, fan_key(identity));
}

#[test]
fn a_short_replica_is_refused_and_written_again() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[9_u8; 8192]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(1).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    let path = root.join(&report.key).join(replica_path(0));
    std::fs::write(&path, [9_u8; 4096]).expect("shorten");
    let identity = digest(&template);
    assert!(open_replica(&template, identity, &root, copies).is_none());
    assert_eq!(warm(&template, &root, copies).expect("rewarm").written, 1);
    assert!(open_replica(&template, identity, &root, copies).is_some());
}

#[test]
fn a_replica_holding_other_bytes_is_written_again() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[1_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(1).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    let path = root.join(&report.key).join(replica_path(0));
    std::fs::write(&path, [2_u8; 4096]).expect("overwrite");
    assert_eq!(warm(&template, &root, copies).expect("rewarm").written, 1);
    assert_eq!(std::fs::read(&path).expect("read"), vec![1_u8; 4096]);
}

#[test]
fn a_changed_template_keys_somewhere_else() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let first = template(scratch.path(), &[1_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(1).expect("nonzero");
    let before = warm(&first, &root, copies).expect("warm");
    let second = template(scratch.path(), &[1_u8; 8192]);
    let after = warm(&second, &root, copies).expect("warm");
    assert_ne!(before.key, after.key);
}

#[test]
fn a_same_sized_artifact_with_another_digest_cannot_reuse_the_fan() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let first = template(scratch.path(), &[1_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(1).expect("nonzero");
    let report = warm(&first, &root, copies).expect("warm");

    std::fs::write(scratch.path().join("template.ext4"), [2_u8; 4096])
        .expect("replace template bytes");
    let second = File::open(scratch.path().join("template.ext4")).expect("reopen template");
    let second_digest = digest(&second);

    assert_ne!(report.key, fan_key(second_digest));
    assert!(open_replica(&second, second_digest, &root, copies).is_none());
}

#[test]
fn the_copy_count_is_clamped() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[5_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(MAX_TEMPLATE_COPIES + 5).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    assert_eq!(report.copies, MAX_TEMPLATE_COPIES);
    assert_eq!(count(&root.join(report.key)), MAX_TEMPLATE_COPIES);
}

#[test]
fn every_replica_has_a_directory_of_its_own() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[4_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(3).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    let parents: std::collections::BTreeSet<PathBuf> = (0..report.copies)
        .map(|index| {
            root.join(&report.key)
                .join(replica_path(index))
                .parent()
                .expect("parent")
                .to_path_buf()
        })
        .collect();
    assert_eq!(parents.len(), report.copies);
    for parent in parents {
        assert!(parent.is_dir());
    }
}
