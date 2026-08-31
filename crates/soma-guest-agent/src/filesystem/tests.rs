//! The filesystem operations exercised against a real temporary directory.
//!
//! Nothing here stubs the filesystem. Every case names a path that exists, or deliberately does
//! not, and asserts the outcome the protocol requires, because the whole value of this module is
//! that it agrees with the kernel about what happened.

mod directory;
mod failures;
mod mode;
mod presence;

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use soma_guest::{FileOutcome, FileRequest, MAX_CHUNK_BYTES};
use tempfile::TempDir;

use super::perform;

/// Names a path the way the protocol carries one.
fn at(path: &Path) -> Box<[u8]> {
    path.as_os_str().as_bytes().into()
}

/// Reads a whole chunk from one path.
fn read(path: &Path, offset: u64, length: u32) -> FileOutcome {
    perform(&FileRequest::Read {
        path: at(path),
        offset,
        length,
    })
}

/// Writes one chunk to one path.
fn write(path: &Path, offset: u64, create: bool, shorten: bool, bytes: &[u8]) -> FileOutcome {
    perform(&FileRequest::Write {
        path: at(path),
        offset,
        create,
        shorten,
        bytes: bytes.into(),
    })
}

/// Returns the bytes of a `Read` outcome and whether it reached the end.
fn read_parts(outcome: &FileOutcome) -> (Vec<u8>, bool) {
    match outcome {
        FileOutcome::Read { bytes, end } => (bytes.to_vec(), *end),
        other => panic!("expected a read outcome, got {other:?}"),
    }
}

#[test]
fn reads_the_bytes_at_the_requested_offset() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"abcdefghij").expect("seed file");

    let (bytes, end) = read_parts(&read(&path, 3, 4));

    assert_eq!(bytes, b"defg");
    assert!(!end, "four of ten bytes is not the end of the file");
}

#[test]
fn reports_the_end_when_the_read_reaches_it() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"abcdefghij").expect("seed file");

    let (bytes, end) = read_parts(&read(&path, 6, 64));

    assert_eq!(bytes, b"ghij");
    assert!(end, "the read consumed the last byte");
}

#[test]
fn reads_nothing_past_the_end_of_the_file() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"abc").expect("seed file");

    let (bytes, end) = read_parts(&read(&path, 100, 16));

    assert!(bytes.is_empty());
    assert!(end, "an offset past the end has already reached it");
}

#[test]
fn a_read_that_ends_exactly_on_the_last_byte_reports_the_end() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"abcd").expect("seed file");

    let (bytes, end) = read_parts(&read(&path, 0, 4));

    assert_eq!(bytes, b"abcd");
    assert!(end, "the request asked for exactly the whole file");
}

#[test]
fn caps_a_read_at_what_one_record_can_carry() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("large");
    fs::write(&path, vec![7_u8; MAX_CHUNK_BYTES + 16]).expect("seed file");

    let (bytes, end) = read_parts(&read(&path, 0, u32::MAX));

    assert_eq!(bytes.len(), MAX_CHUNK_BYTES);
    assert!(!end, "a capped read leaves the caller more to ask for");
}

#[test]
fn writes_bytes_at_the_requested_offset() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"aaaaaa").expect("seed file");

    let outcome = write(&path, 2, false, false, b"bb");

    assert_eq!(outcome, FileOutcome::Written { bytes: 2 });
    assert_eq!(fs::read(&path).expect("read back"), b"aabbaa");
}

#[test]
fn creates_the_file_only_when_the_request_asks() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("fresh");

    let outcome = write(&path, 0, true, false, b"hello");

    assert_eq!(outcome, FileOutcome::Written { bytes: 5 });
    assert_eq!(fs::read(&path).expect("read back"), b"hello");
}

#[test]
fn shortening_ends_the_file_where_the_write_ends() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"aaaaaaaaaa").expect("seed file");

    let outcome = write(&path, 2, false, true, b"bb");

    assert_eq!(outcome, FileOutcome::Written { bytes: 2 });
    assert_eq!(fs::read(&path).expect("read back"), b"aabb");
}

#[test]
fn a_write_without_shortening_leaves_the_tail_alone() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"aaaaaaaaaa").expect("seed file");

    let outcome = write(&path, 2, false, false, b"bb");

    assert_eq!(outcome, FileOutcome::Written { bytes: 2 });
    assert_eq!(fs::read(&path).expect("read back"), b"aabbaaaaaa");
}

#[test]
fn a_write_past_the_end_extends_the_file() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"ab").expect("seed file");

    let outcome = write(&path, 4, false, false, b"cd");

    assert_eq!(outcome, FileOutcome::Written { bytes: 2 });
    assert_eq!(fs::read(&path).expect("read back"), b"ab\0\0cd");
}
