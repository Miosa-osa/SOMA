//! What a recording backend keeps, and that a plain one keeps nothing.
//!
//! The scratch file is one fixed name at one fixed size in the system temp directory, created
//! once per run and deliberately never removed: a test that cleaned up after itself would be a
//! file deletion on the machine these run on. One fixed name means repeated runs reuse it rather
//! than accumulating, and one fixed size means the tests, which run in parallel, cannot resize
//! each other's file.

use std::fs::{File, OpenOptions};
use std::io::{Seek as _, SeekFrom, Write as _};

use super::{BackendError, BlockBackend, FileBackend};
use crate::head_reads;

/// One megabyte, fixed, so no test can shrink the file another is reading.
const SCRATCH_BYTES: u64 = 1 << 20;

/// The scratch head, sized and holding one recognizable page.
fn head() -> File {
    let path = std::env::temp_dir().join("soma-kvm-head-reads.img");
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .expect("scratch head");
    file.set_len(SCRATCH_BYTES).expect("size the scratch head");
    file.seek(SeekFrom::Start(0)).expect("rewind");
    file.write_all(&[0x5a_u8; 4096]).expect("write a page");
    file.flush().expect("flush");
    file.seek(SeekFrom::Start(0)).expect("rewind for reading");
    file
}

#[test]
fn the_handle_a_session_holds_sees_what_the_device_read() {
    let handle = head_reads::recording();
    let mut backend = FileBackend::recording(head(), false, std::sync::Arc::clone(&handle))
        .expect("a recording backend");
    let mut buf = [0_u8; 4096];
    backend.read_at(0x1000, &mut buf).expect("a page");
    backend.read_at(0x2000, &mut buf).expect("the next page");
    assert_eq!(
        head_reads::seal(&handle),
        vec![head_reads::HeadSpan {
            offset: 0x1000,
            len: 8192
        }],
        "two adjacent pages are one range, and the session holds what the device recorded"
    );
}

#[test]
fn a_read_the_device_refused_is_not_part_of_the_warm_set() {
    let handle = head_reads::recording();
    let mut backend = FileBackend::recording(head(), false, std::sync::Arc::clone(&handle))
        .expect("a recording backend");
    let mut buf = [0_u8; 4096];
    backend.read_at(0, &mut buf).expect("the first page");
    let refused = backend.read_at(SCRATCH_BYTES, &mut buf);
    assert_eq!(refused, Err(BackendError::OutOfRange));
    assert_eq!(
        head_reads::seal(&handle),
        vec![head_reads::HeadSpan {
            offset: 0,
            len: 4096
        }],
        "the warm set holds what the guest got, not what it asked for"
    );
}

#[test]
fn a_plain_backend_still_serves_reads() {
    let mut backend = FileBackend::new(head(), false).expect("a plain backend");
    let mut buf = [0_u8; 4096];
    assert_eq!(backend.read_at(0, &mut buf).expect("a page"), 4096);
    assert_eq!(buf[0], 0x5a, "the page came back");
}

#[test]
fn a_sealed_session_stops_the_set_growing() {
    let handle = head_reads::recording();
    let mut backend = FileBackend::recording(head(), false, std::sync::Arc::clone(&handle))
        .expect("a recording backend");
    let mut buf = [0_u8; 4096];
    backend.read_at(0, &mut buf).expect("the launch's own page");
    let at_ready = head_reads::seal(&handle);
    backend
        .read_at(8192, &mut buf)
        .expect("a read the guest chose to make later");
    assert_eq!(at_ready.len(), 1);
    assert!(
        head_reads::seal(&handle).is_empty(),
        "a read after Ready is a request, not a cost of the launch"
    );
}
