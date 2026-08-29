use std::io::{Cursor, Read};

use sha2::{Digest as _, Sha256};

use crate::{ImportErrorKind, ImportPhase, digest};

use super::{MAX_ENTRIES, MAX_PATH_BYTES, MAX_PATH_METADATA_BYTES, PathPolicy, validate};

#[test]
fn validates_a_chunked_archive_and_reports_its_exact_stream_identity() {
    let mut archive = archive_with(&[(b"usr/bin/node", b"node"), (b"etc/os-release", b"linux")]);
    archive.extend_from_slice(&[0_u8; 1_024]);
    let expected = digest::from_output(Sha256::digest(&archive).as_ref());
    let mut reader = Chunked::new(&archive, 17);

    let validated = validate(&mut reader, u64::try_from(archive.len()).unwrap()).unwrap();

    assert_eq!(validated.diff_id, expected);
    assert_eq!(validated.expanded_size, archive.len() as u64);
    assert_eq!(validated.entry_count, 2);
}

#[test]
fn rejects_bad_checksum_and_truncated_structure() {
    let mut checksum = archive_with(&[(b"file", b"contents")]);
    checksum[0] ^= 1;
    let mut malformed_size = archive_with(&[(b"file", b"contents")]);
    rewrite_first_header(&mut malformed_size, |header| {
        header.as_mut_bytes()[124..136].fill(b'z');
    });
    let payload = archive_with(&[(b"file", b"contents")]);
    let cases = [
        checksum,
        malformed_size,
        vec![0_u8; 511],
        payload[..514].to_vec(),
    ];

    for bytes in cases {
        assert_error(&bytes, bytes.len() as u64, ImportErrorKind::Integrity);
    }
}

#[test]
fn rejects_missing_or_nonzero_termination() {
    let archive = archive_with(&[(b"file", b"contents")]);
    let missing_second_zero_block = archive[..archive.len() - 512].to_vec();
    let mut nonzero_tail = archive.clone();
    *nonzero_tail.last_mut().unwrap() = 1;

    for bytes in [missing_second_zero_block, nonzero_tail] {
        assert_error(&bytes, bytes.len() as u64, ImportErrorKind::Integrity);
    }
}

#[test]
fn rejects_duplicate_absolute_and_parent_traversal_paths() {
    let duplicate = archive_with(&[(b"usr/bin/node", b"one"), (b"./usr//bin/node", b"two")]);
    let absolute = archive_with(&[(b"/etc/passwd", b"root")]);
    let traversal = archive_with(&[(b"usr/../etc/passwd", b"root")]);

    for bytes in [duplicate, absolute, traversal] {
        assert_error(&bytes, bytes.len() as u64, ImportErrorKind::InvalidInput);
    }
}

#[test]
fn rejects_escaping_hard_links_but_accepts_linux_symlink_targets() {
    for target in [b"/outside".as_slice(), b"../../outside".as_slice()] {
        let hard_link = archive_with_link(tar::EntryType::Link, target);
        assert_error(
            &hard_link,
            hard_link.len() as u64,
            ImportErrorKind::InvalidInput,
        );
    }

    for target in [b"/proc/mounts".as_slice(), b"../lib/library.so".as_slice()] {
        let symlink = archive_with_link(tar::EntryType::Symlink, target);
        validate(&mut Cursor::new(&symlink), symlink.len() as u64).unwrap();
    }
}

#[test]
fn rejects_overlong_effective_gnu_path_and_enforces_stream_limit() {
    let overlong = archive_with_long_name(&vec![b'a'; MAX_PATH_BYTES + 1]);
    assert_error(
        &overlong,
        overlong.len() as u64,
        ImportErrorKind::LimitExceeded,
    );

    let valid = archive_with(&[(b"file", b"contents")]);
    assert_error(
        &valid,
        valid.len() as u64 - 1,
        ImportErrorKind::LimitExceeded,
    );
}

#[test]
fn entry_and_aggregate_path_metadata_usage_are_bounded() {
    let mut entries = PathPolicy {
        entry_count: MAX_ENTRIES,
        ..PathPolicy::default()
    };
    assert_eq!(
        entries.observe(b"next", None, false).unwrap_err(),
        super::limit_error()
    );

    let mut metadata = PathPolicy {
        path_metadata_bytes: MAX_PATH_METADATA_BYTES,
        ..PathPolicy::default()
    };
    assert_eq!(
        metadata.observe(b"next", None, false).unwrap_err(),
        super::limit_error()
    );
}

fn assert_error(bytes: &[u8], maximum: u64, kind: ImportErrorKind) {
    let error = validate(&mut Cursor::new(bytes), maximum).unwrap_err();
    assert_eq!(error.phase(), ImportPhase::VerifyLayer);
    assert_eq!(error.kind(), kind);
}

fn archive_with(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut archive);
        for (path, contents) in entries {
            let mut header = tar::Header::new_ustar();
            set_raw_name(&mut header, path);
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, *contents).unwrap();
        }
        builder.finish().unwrap();
    }
    archive
}

fn archive_with_long_name(path: &[u8]) -> Vec<u8> {
    let mut archive = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut archive);
        let mut extension = tar::Header::new_gnu();
        extension.set_entry_type(tar::EntryType::GNULongName);
        extension.set_size((path.len() + 1) as u64);
        extension.set_mode(0o644);
        extension.set_cksum();
        let mut body = path.to_vec();
        body.push(0);
        builder.append(&extension, body.as_slice()).unwrap();

        let mut header = tar::Header::new_gnu();
        set_raw_name(&mut header, b"placeholder");
        header.set_size(0);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
        builder.finish().unwrap();
    }
    archive
}

fn archive_with_link(kind: tar::EntryType, target: &[u8]) -> Vec<u8> {
    let mut archive = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut archive);
        let mut header = tar::Header::new_ustar();
        set_raw_name(&mut header, b"link");
        header.set_entry_type(kind);
        header.set_size(0);
        header.set_mode(0o777);
        let link = &mut header.as_mut_bytes()[157..257];
        link.fill(0);
        link[..target.len()].copy_from_slice(target);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
        builder.finish().unwrap();
    }
    archive
}

fn set_raw_name(header: &mut tar::Header, path: &[u8]) {
    assert!(path.len() <= 100);
    let name = &mut header.as_mut_bytes()[..100];
    name.fill(0);
    name[..path.len()].copy_from_slice(path);
}

fn rewrite_first_header(archive: &mut [u8], rewrite: impl FnOnce(&mut tar::Header)) {
    let mut header = tar::Header::new_old();
    header.as_mut_bytes().copy_from_slice(&archive[..512]);
    rewrite(&mut header);
    header.set_cksum();
    archive[..512].copy_from_slice(header.as_bytes());
}

struct Chunked<'a> {
    inner: Cursor<&'a [u8]>,
    maximum_chunk: usize,
}

impl<'a> Chunked<'a> {
    fn new(bytes: &'a [u8], maximum_chunk: usize) -> Self {
        Self {
            inner: Cursor::new(bytes),
            maximum_chunk,
        }
    }
}

impl Read for Chunked<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let length = buffer.len().min(self.maximum_chunk);
        self.inner.read(&mut buffer[..length])
    }
}
