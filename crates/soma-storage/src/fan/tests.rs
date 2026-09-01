//! Fan behaviour that does not need a reflink filesystem.

use std::num::NonZeroUsize;

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
        .count()
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
        let path = root.join(&report.key).join(replica_name(index));
        assert_eq!(std::fs::read(&path).expect("read replica"), bytes);
    }
}

#[test]
fn a_replica_is_opened_only_when_the_fan_is_there() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[3_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(2).expect("nonzero");
    assert!(open_replica(&template, &root, copies).is_none());
    let report = warm(&template, &root, copies).expect("warm");
    let replica = open_replica(&template, &root, copies).expect("replica");
    assert_eq!(replica.metadata().expect("metadata").size(), 4096);
    assert_ne!(
        replica.metadata().expect("metadata").ino(),
        template.metadata().expect("metadata").ino()
    );
    assert_eq!(report.key, fan_key(&template).expect("key"));
}

#[test]
fn a_short_replica_is_refused_and_written_again() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[9_u8; 8192]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(1).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    let path = root.join(&report.key).join(replica_name(0));
    std::fs::write(&path, [9_u8; 4096]).expect("shorten");
    assert!(open_replica(&template, &root, copies).is_none());
    assert_eq!(warm(&template, &root, copies).expect("rewarm").written, 1);
    assert!(open_replica(&template, &root, copies).is_some());
}

#[test]
fn a_replica_holding_other_bytes_is_written_again() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[1_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(1).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    let path = root.join(&report.key).join(replica_name(0));
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
fn the_copy_count_is_clamped() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let template = template(scratch.path(), &[5_u8; 4096]);
    let root = scratch.path().join("fan");
    let copies = NonZeroUsize::new(MAX_TEMPLATE_COPIES + 5).expect("nonzero");
    let report = warm(&template, &root, copies).expect("warm");
    assert_eq!(report.copies, MAX_TEMPLATE_COPIES);
    assert_eq!(count(&root.join(report.key)), MAX_TEMPLATE_COPIES);
}
