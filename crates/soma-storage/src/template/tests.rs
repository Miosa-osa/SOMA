use std::path::Path;

use super::recipe::{self, FAKE_TIME_EPOCH, MKE2FS_CONFIG, TOOL_PATH};
use super::*;
use crate::profile::{
    BlockSize, ClassName, Ext4FeatureSet, InodePolicy, LogicalBytes, MountOption, MountOptions,
    UuidPolicy,
};

fn recipe_for(name: &str, version: u32, uuid_policy: UuidPolicy) -> OverlayRecipe {
    OverlayRecipe {
        name: ClassName::new(name).expect("class name"),
        version,
        logical_bytes: LogicalBytes::new(64 * 1024 * 1024, BlockSize::B4096).expect("size"),
        block_size: BlockSize::B4096,
        uuid_policy,
        features: Ext4FeatureSet::V1,
        inode_policy: InodePolicy::bytes_per_inode(16384).expect("ratio"),
        mount_options: MountOptions::new(&[MountOption::NoAtime]),
    }
}

#[test]
fn identity_is_deterministic_and_changes_with_name_version_and_size() {
    let base = recipe_for("ovl", 1, UuidPolicy::Derived);
    let (uuid_a, seed_a) = recipe::derive_identity(&base);
    let (uuid_b, seed_b) = recipe::derive_identity(&base);
    assert_eq!(uuid_a, uuid_b);
    assert_eq!(seed_a, seed_b);
    assert_ne!(uuid_a, seed_a);
    assert_eq!(uuid_a.as_bytes()[6] & 0xf0, 0x80);
    assert_eq!(uuid_a.as_bytes()[8] & 0xc0, 0x80);
    assert_eq!(uuid_a.render().len(), 36);

    let other_version = recipe_for("ovl", 2, UuidPolicy::Derived);
    assert_ne!(recipe::derive_identity(&other_version).0, uuid_a);
    let other_name = recipe_for("ovl2", 1, UuidPolicy::Derived);
    assert_ne!(recipe::derive_identity(&other_name).0, uuid_a);

    let explicit = recipe_for("ovl", 1, UuidPolicy::Explicit([0x11; 16]));
    let (uuid_c, seed_c) = recipe::derive_identity(&explicit);
    assert_eq!(uuid_c.as_bytes(), &[0x11; 16]);
    assert_eq!(uuid_c.render(), "11111111-1111-1111-1111-111111111111");
    assert_eq!(seed_c, seed_a);
}

#[test]
fn mke2fs_invocation_is_exact_and_environment_is_closed() {
    let recipe = recipe_for("ovl", 1, UuidPolicy::Derived);
    let (uuid, seed) = recipe::derive_identity(&recipe);
    let invocation = recipe::mke2fs_invocation(
        &recipe,
        Path::new("/store/ovl-v1.ext4"),
        Path::new("/store/c"),
    );
    assert_eq!(invocation.program, "mke2fs");
    let expected: Vec<String> = [
        "-F",
        "-q",
        "-t",
        "ext4",
        "-b",
        "4096",
        "-I",
        "256",
        "-i",
        "16384",
        "-m",
        "0",
        "-r",
        "1",
        "-L",
        "ovl",
        "-U",
        &uuid.render(),
        "-e",
        "remount-ro",
        "-O",
        Ext4FeatureSet::V1.mke2fs_argument(),
        "-E",
        &format!(
            "hash_seed={},lazy_itable_init=0,lazy_journal_init=0,root_owner=0:0,nodiscard",
            seed.render()
        ),
        "/store/ovl-v1.ext4",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(invocation.args, expected);
    assert_eq!(
        invocation.env,
        vec![
            ("PATH".to_owned(), TOOL_PATH.to_owned()),
            (
                "E2FSPROGS_FAKE_TIME".to_owned(),
                FAKE_TIME_EPOCH.to_string()
            ),
            ("MKE2FS_CONFIG".to_owned(), "/store/c".to_owned()),
            ("LC_ALL".to_owned(), "C".to_owned()),
        ]
    );
    assert_eq!(FAKE_TIME_EPOCH, 1_787_961_600);
    assert!(MKE2FS_CONFIG.contains("lazy_itable_init = 0"));
    assert!(MKE2FS_CONFIG.contains("default_mntopts = acl,user_xattr"));
    let long = recipe_for("abcdefghijklmnopqrstuvwxyz", 1, UuidPolicy::Derived);
    assert_eq!(recipe::volume_label(&long), "abcdefghijklmnop");
    assert_eq!(recipe::template_file_name(&recipe), "ovl-v1.ext4");
    let fsck = recipe::e2fsck_invocation(Path::new("/store/ovl-v1.ext4"));
    assert_eq!(fsck.program, "e2fsck");
    assert_eq!(fsck.args, vec!["-f", "-n", "/store/ovl-v1.ext4"]);
}

#[test]
fn digest_file_matches_sha256_of_the_bytes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("bytes");
    std::fs::write(&path, b"soma").expect("write");
    let digest = digest_file(&path).expect("digest");
    assert_eq!(digest.to_string(), expected_sha256(b"soma"));
    assert!(digest_file(temp.path().join("missing").as_path()).is_err());
}

fn expected_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let mut text = String::new();
    for byte in Sha256::digest(bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[test]
fn missing_store_directory_fails_before_any_tool_runs() {
    let recipe = recipe_for("ovl", 1, UuidPolicy::Derived);
    let error =
        create_template(Path::new("/nonexistent/soma-store"), &recipe).expect_err("missing");
    assert!(matches!(error, TemplateError::Create(_)), "{error}");
}

#[test]
fn existing_template_is_never_overwritten() {
    let temp = tempfile::tempdir().expect("tempdir");
    let recipe = recipe_for("ovl", 1, UuidPolicy::Derived);
    std::fs::write(temp.path().join("ovl-v1.ext4"), b"existing").expect("write");
    let error = create_template(temp.path(), &recipe).expect_err("exists");
    assert!(
        matches!(error, TemplateError::Create(ref e) if e.kind() == io::ErrorKind::AlreadyExists),
        "{error}"
    );
    assert_eq!(
        std::fs::read(temp.path().join("ovl-v1.ext4")).expect("read"),
        b"existing"
    );
}

/// Requires `mke2fs` and `e2fsck` from e2fsprogs 1.47 on `PATH`; runs on any filesystem.
#[test]
#[ignore = "requires mke2fs and e2fsck from e2fsprogs on the host"]
fn two_templates_from_one_recipe_are_byte_identical() {
    let temp_a = tempfile::tempdir().expect("tempdir");
    let temp_b = tempfile::tempdir().expect("tempdir");
    let recipe = recipe_for("ovl", 1, UuidPolicy::Derived);
    let a = create_template(temp_a.path(), &recipe).expect("template a");
    let b = create_template(temp_b.path(), &recipe).expect("template b");
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.logical_bytes(), 64 * 1024 * 1024);
    assert_eq!(a.mke2fs().program, "mke2fs");
    assert_eq!(
        std::fs::read(a.path()).expect("a"),
        std::fs::read(b.path()).expect("b")
    );
    assert!(!temp_a.path().join("ovl-v1.mke2fs.conf").exists());
    assert_eq!(
        a.open().expect("open").metadata().expect("meta").len(),
        64 * 1024 * 1024
    );
}
