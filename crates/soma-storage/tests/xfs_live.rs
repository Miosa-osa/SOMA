//! Live conformance tests on a real XFS `reflink=1` mount.
//!
//! Every test is ignored by default and fails loudly, never silently passes, when the
//! prerequisite environment is missing:
//!
//! - `SOMA_XFS_REFLINK_DIR`: a writable directory on XFS with `reflink=1`.
//! - `SOMA_XFS_TEMPLATE_DIR`: a writable directory on the same filesystem for templates.
//! - `SOMA_XFS_TINY_DIR`: a writable directory on a small XFS `reflink=1` filesystem that a
//!   64 MiB template can exhaust.
//! - `SOMA_XFS_NOREFLINK_DIR`: a writable directory on XFS with `reflink=0`.
//!
//! `scripts/xfs-reflink-bench.sh` provides all four inside a privileged container.

#![cfg(target_os = "linux")]

use std::fs::File;
use std::os::fd::AsFd;
use std::path::PathBuf;

use soma_storage::clone::{self, CloneError};
use soma_storage::head::{HeadName, HeadToken};
use soma_storage::lease::HeadLedger;
use soma_storage::profile::{
    BlockSize, ClassName, Ext4FeatureSet, InodePolicy, LogicalBytes, MountOptions, OverlayRecipe,
    ProfileRejection, StorageProfile, UuidPolicy,
};
use soma_storage::reconcile::{self, Disposition};
use soma_storage::release::{self, ReleaseOutcome};
use soma_storage::template::{self, SterileTemplate};
use soma_storage::verify;

fn required_dir(name: &str) -> PathBuf {
    let value = std::env::var(name).unwrap_or_else(|_| {
        panic!("prerequisite missing: set {name} to a directory on the required XFS mount")
    });
    let path = PathBuf::from(value);
    assert!(
        path.is_dir(),
        "prerequisite missing: {name}={} is not a directory",
        path.display()
    );
    path
}

fn open_dir(path: &PathBuf) -> File {
    File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

fn recipe(name: &str, mib: u64) -> OverlayRecipe {
    OverlayRecipe {
        name: ClassName::new(name).expect("class name"),
        version: 1,
        logical_bytes: LogicalBytes::new(mib * 1024 * 1024, BlockSize::B4096).expect("size"),
        block_size: BlockSize::B4096,
        uuid_policy: UuidPolicy::Derived,
        features: Ext4FeatureSet::V1,
        inode_policy: InodePolicy::bytes_per_inode(16384).expect("ratio"),
        mount_options: MountOptions::new(&[]),
    }
}

/// Creates or reuses the live template `name` of `mib` MiB inside the template directory.
fn template(name: &str, mib: u64) -> SterileTemplate {
    let store = required_dir("SOMA_XFS_TEMPLATE_DIR");
    let recipe = recipe(name, mib);
    let path = store.join(format!("{name}-v1.ext4"));
    if path.exists() {
        std::fs::remove_file(&path).expect("remove stale template");
    }
    template::create_template(&store, &recipe).unwrap_or_else(|e| panic!("create template: {e}"))
}

fn token(byte: u8) -> HeadToken {
    HeadToken::new([byte; 16]).expect("non-zero")
}

#[test]
#[ignore = "requires SOMA_XFS_REFLINK_DIR on XFS with reflink=1"]
fn profile_probe_accepts_the_reflink_mount() {
    let dir = open_dir(&required_dir("SOMA_XFS_REFLINK_DIR"));
    let profile = StorageProfile::probe(dir.as_fd()).unwrap_or_else(|e| panic!("probe: {e}"));
    assert!(profile.free_bytes() > 0);
    assert!(profile.mount_id() > 0);
    assert_eq!(profile.block_size(), 4096);
}

#[test]
#[ignore = "requires SOMA_XFS_NOREFLINK_DIR on XFS with reflink=0"]
fn profile_probe_rejects_the_reflink_disabled_mount_and_so_does_clone() {
    let dir = open_dir(&required_dir("SOMA_XFS_NOREFLINK_DIR"));
    match StorageProfile::probe(dir.as_fd()) {
        Err(ProfileRejection::ReflinkUnsupported) => {}
        other => panic!("expected ReflinkUnsupported, got {other:?}"),
    }
    let probe = std::fs::read_dir(required_dir("SOMA_XFS_NOREFLINK_DIR")).expect("read dir");
    assert_eq!(probe.count(), 0, "probe files must be unlinked");

    let noreflink = required_dir("SOMA_XFS_NOREFLINK_DIR");
    let recipe = recipe("live-noreflink", 64);
    let template = template::create_template(&noreflink, &recipe).expect("template on reflink=0");
    let template_file = template.open().expect("open");
    let name = HeadName::new("live-noreflink-head").expect("name");
    match clone::clone_head(template_file.as_fd(), dir.as_fd(), &name) {
        Err(CloneError::ReflinkUnsupported) => {}
        other => panic!("expected ReflinkUnsupported, got {other:?}"),
    }
    assert!(
        !noreflink.join(name.as_str()).exists(),
        "failed clone must be unlinked"
    );
    std::fs::remove_file(template.path()).expect("remove template");
}

#[test]
#[ignore = "requires SOMA_XFS_REFLINK_DIR and SOMA_XFS_TEMPLATE_DIR on XFS with reflink=1"]
fn clone_shares_every_extent_and_isolation_holds_across_two_clones() {
    let heads = open_dir(&required_dir("SOMA_XFS_REFLINK_DIR"));
    let template = template("live-iso", 64);
    let template_file = template.open().expect("open");
    let name = HeadName::new("live-iso-single").expect("name");
    let head = clone::clone_head(template_file.as_fd(), heads.as_fd(), &name).expect("clone");
    assert_eq!(head.apparent_bytes(), 64 * 1024 * 1024);
    assert!(head.extents().all_shared());
    assert!(head.extents().extents > 0);
    match clone::clone_head(template_file.as_fd(), heads.as_fd(), &name) {
        Err(CloneError::AlreadyExists) => {}
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
    drop(head);

    let prefix = HeadName::new("live-iso").expect("prefix");
    let proof =
        verify::prove_isolation(template_file.as_fd(), heads.as_fd(), &prefix).expect("isolation");
    assert_eq!(proof.regions, 4);
    assert!(proof.before.all_shared());
    assert!(proof.after.shared_extents < proof.after.extents);
    assert_eq!(
        template::digest_file(template.path()).expect("digest"),
        template.digest()
    );

    let mut ledger = HeadLedger::new();
    ledger.lease(token(1), name.clone()).expect("lease");
    let outcome = release::release_head(&mut ledger, heads.as_fd(), token(1)).expect("release");
    assert_eq!(outcome, ReleaseOutcome::Destroyed(name));
    let report = reconcile::reconcile(&ledger, heads.as_fd()).expect("reconcile");
    assert!(
        report
            .dispositions()
            .iter()
            .all(|d| !matches!(d, Disposition::Missing { .. }))
    );
}

#[test]
#[ignore = "requires SOMA_XFS_TINY_DIR on a small XFS filesystem with reflink=1"]
fn writing_through_a_clone_reports_enospc_and_leaves_the_template_intact() {
    let tiny = required_dir("SOMA_XFS_TINY_DIR");
    let dir = open_dir(&tiny);
    let recipe = recipe("live-tiny", 256);
    let path = tiny.join("live-tiny-v1.ext4");
    if path.exists() {
        std::fs::remove_file(&path).expect("remove stale template");
    }
    let template = template::create_template(&tiny, &recipe).expect("tiny template");
    let template_file = template.open().expect("open");
    let name = HeadName::new("live-tiny-head").expect("name");
    let proof = verify::prove_no_space(
        template_file.as_fd(),
        dir.as_fd(),
        &name,
        4 * 1024 * 1024 * 1024,
    )
    .unwrap_or_else(|e| panic!("no-space proof: {e}"));
    assert!(proof.bytes_written > 0);
    assert!(!tiny.join(name.as_str()).exists());
    assert_eq!(
        template::digest_file(template.path()).expect("digest"),
        template.digest()
    );
    std::fs::remove_file(template.path()).expect("remove template");
}

#[test]
#[ignore = "requires SOMA_XFS_REFLINK_DIR and SOMA_XFS_TEMPLATE_DIR on XFS with reflink=1"]
fn concurrent_create_and_cleanup_leave_a_clean_directory() {
    let heads = open_dir(&required_dir("SOMA_XFS_REFLINK_DIR"));
    let template = template("live-burst", 64);
    let template_file = template.open().expect("open");
    let names: Vec<HeadName> = (1..=32u8).map(|i| token(i).head_name()).collect();
    let mut ledger = HeadLedger::new();
    for (i, name) in names.iter().enumerate() {
        ledger
            .lease(token(u8::try_from(i + 1).expect("small")), name.clone())
            .expect("lease");
    }
    std::thread::scope(|scope| {
        for name in &names {
            let template_file = &template_file;
            let heads = &heads;
            scope.spawn(move || {
                let head =
                    clone::clone_head(template_file.as_fd(), heads.as_fd(), name).expect("clone");
                assert!(head.extents().all_shared());
            });
        }
    });
    let report = reconcile::reconcile(&ledger, heads.as_fd()).expect("reconcile");
    assert!(report.is_clean(), "{report:?}");
    let heads_path = required_dir("SOMA_XFS_REFLINK_DIR");
    std::thread::scope(|scope| {
        for name in &names {
            let heads = &heads;
            let template_file = &template_file;
            let heads_path = &heads_path;
            scope.spawn(move || {
                let extra = HeadName::new(format!("{name}-x")).expect("name");
                let head =
                    clone::clone_head(template_file.as_fd(), heads.as_fd(), &extra).expect("clone");
                drop(head);
                std::fs::remove_file(heads_path.join(extra.as_str())).expect("unlink");
            });
        }
    });
    for i in 1..=32u8 {
        release::release_head(&mut ledger, heads.as_fd(), token(i)).expect("release");
    }
    let report = reconcile::reconcile(&ledger, heads.as_fd()).expect("reconcile");
    assert!(report.is_clean(), "{report:?}");
}
