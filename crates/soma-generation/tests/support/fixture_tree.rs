#![allow(dead_code)]

#[cfg(unix)]
use std::process::Command;
use std::{fs, path::Path};

use soma_generation::{ArtifactRole, CompiledCandidate};

use super::rootfs::{TarEntry, local_pax_layer, tar};

pub const LONG_NAME: &[u8] =
    b"long/component-name-that-exceeds-the-one-hundred-byte-ustar-field-and-forces-a-pax-path-record-0123456789";
pub const AGENT: &[u8] = b"synthetic-guest-agent";

pub fn big_body() -> Vec<u8> {
    (0..10_240_u32).map(|value| (value % 251) as u8).collect()
}

pub fn fixture_layers() -> Vec<Vec<u8>> {
    let big = big_body();
    let exact = vec![0xab_u8; 4096];
    let first = tar(&[
        TarEntry::directory(b"etc")
            .mode(0o750)
            .ownership(5, 7)
            .mtime(1_500_000_000),
        TarEntry::file(b"etc/a", b"alpha").mtime(1_400_000_000),
        TarEntry::hardlink(b"etc/a-hard", b"etc/a"),
        TarEntry::file(b"etc/z", b"zulu")
            .mode(0o4755)
            .ownership(3_000_000, 80_000),
        TarEntry::symlink(b"a-link", b"../etc/a"),
        TarEntry::fifo(b"pipe").mode(0o600),
        TarEntry::directory(b"tmp").mode(0o1777),
        TarEntry::file(b"big", &big),
        TarEntry::file(b"exact", &exact),
        TarEntry::file(b"empty", b""),
    ]);
    let second = tar(&[
        TarEntry::directory(b"usr"),
        TarEntry::directory(b"usr/bin"),
        TarEntry::file(b"usr/bin/x", b"#!/bin/sh\n").mode(0o755),
        TarEntry::hardlink(b"usr/bin/x-hard", b"usr/bin/x"),
        TarEntry::directory(b"long"),
    ]);
    let third = local_pax_layer(&TarEntry::file(b"long/x", b"pax"), &[("path", LONG_NAME)]);
    vec![first, second, third]
}

#[cfg(unix)]
pub fn extraction_oracle(erofs_tools: &Path, store: &Path, compiled: &CompiledCandidate) {
    let root = &compiled.candidate.manifest.root.descriptor;
    let image = store
        .join("v1/blobs/sha256")
        .join(&root.digest.to_string()[7..]);
    let extract = tempfile::tempdir().unwrap();
    let target = extract.path().join("tree");
    let status = Command::new(erofs_tools.join("fsck.erofs"))
        .arg(format!("--extract={}", target.display()))
        .arg("--preserve-perms")
        .arg(&image)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read(target.join("etc/a")).unwrap(), b"alpha");
    assert_eq!(fs::read(target.join("etc/a-hard")).unwrap(), b"alpha");
    assert_eq!(fs::read(target.join("big")).unwrap(), big_body());
    assert_eq!(fs::read(target.join("exact")).unwrap(), vec![0xab_u8; 4096]);
    assert_eq!(fs::read(target.join("empty")).unwrap(), b"");
    assert_eq!(
        fs::read(target.join(std::str::from_utf8(LONG_NAME).unwrap())).unwrap(),
        b"pax"
    );
    assert_eq!(
        fs::read_link(target.join("a-link")).unwrap(),
        Path::new("../etc/a")
    );
    let pipe = fs::symlink_metadata(target.join("pipe")).unwrap();
    assert!(std::os::unix::fs::FileTypeExt::is_fifo(&pipe.file_type()));
    let count = walk_count(&target);
    assert_eq!(count, u64::from(compiled.erofs.entries_verified) - 1);
}

pub fn walk_count(path: &Path) -> u64 {
    let mut count = 0;
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        count += 1;
        if entry.file_type().unwrap().is_dir() {
            count += walk_count(&entry.path());
        }
    }
    count
}

pub fn digests(compiled: &CompiledCandidate) -> Vec<(ArtifactRole, String, u64)> {
    compiled
        .candidate
        .manifest
        .descriptors()
        .iter()
        .map(|descriptor| {
            (
                descriptor.role,
                descriptor.digest.to_string(),
                descriptor.size,
            )
        })
        .collect()
}
