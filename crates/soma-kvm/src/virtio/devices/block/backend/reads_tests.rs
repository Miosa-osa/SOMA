//! What a recording backend keeps, and that a plain one keeps nothing.
//!
//! Every case reads a scratch head of its own: an anonymous temporary file with no name in the
//! file system. No two cases or runs can share it, nothing is left behind for anyone to remove,
//! and a file some other user left under a fixed name can no longer stop a run, which a
//! root-owned `soma-kvm-head-reads.img` did on a shared check host.

use std::fs::File;
use std::io::{Seek as _, SeekFrom, Write as _};

use super::{BackendError, BlockBackend, FileBackend};
use crate::head_reads;

/// The size of every scratch head: one megabyte.
const SCRATCH_BYTES: u64 = 1 << 20;

/// A scratch head of this case's own, sized and holding one recognizable page.
fn head() -> File {
    let mut file = tempfile::tempfile().expect("scratch head");
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
