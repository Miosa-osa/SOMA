//! Copying a writable class without materializing its holes.
//!
//! A writable class is a sparse ext4 image: a forty-gigabyte head carries tens of megabytes of
//! metadata, and the rest is holes. A byte-for-byte copy fills every one of those holes with
//! allocated blocks, so one live run costs a whole writable class of disk for its template and
//! another for its private head. That is enough to make the large shape unaffordable on a host
//! that can actually run it: forty gigabytes of head is not a property of the machine, it is a
//! property of the copy.
//!
//! Two things make a class cost less than its length. The kernel can say where a file's data is,
//! so the walk follows the source's data extents and leaves its holes alone; and a chunk that is
//! all zeros is skipped as well, because a class that was written out rather than left sparse has
//! no holes to find but is still almost entirely zeros. Skipping either one is safe for the same
//! reason: the destination is a fresh file, so a byte this never writes is a zero byte, which is
//! exactly what the source had. The destination ends up the same length and the same bytes, which
//! is what the guest sees either way.

#![allow(unsafe_code)]

use std::{
    fs::File,
    io,
    os::{
        fd::AsRawFd as _,
        unix::fs::{FileExt as _, MetadataExt as _},
    },
};

/// Bytes moved per read and write.
const CHUNK: usize = 1024 * 1024;

/// Copies `source` into `destination` up to `length`, skipping holes and zero chunks.
///
/// The destination must already exist, be writable, and be **empty**: the bytes this skips are
/// left as the zeros a fresh file has, so a destination that already carried data would keep it.
/// Its length is set to `length` whether or not the source had data at the end, so a caller
/// comparing lengths compares what it asked for. Returns the bytes actually written.
///
/// # Errors
///
/// Returns the read, seek, or write failure.
pub fn copy_sparse(source: &File, destination: &File, length: u64) -> io::Result<u64> {
    let mut buffer = vec![0_u8; CHUNK];
    let mut offset = 0_u64;
    let mut written = 0_u64;
    while offset < length {
        // SAFETY: both calls take a live descriptor and an offset the loop keeps inside the file.
        let data =
            unsafe { libc::lseek(source.as_raw_fd(), offset as libc::off_t, libc::SEEK_DATA) };
        if data < 0 {
            // `ENXIO` means there is no further data: everything left is a hole, and the
            // destination already has it, because it was created empty and only written where the
            // source had bytes.
            break;
        }
        let hole = unsafe { libc::lseek(source.as_raw_fd(), data, libc::SEEK_HOLE) };
        if hole < 0 {
            return Err(io::Error::last_os_error());
        }
        let end = u64::try_from(hole).unwrap_or(length);
        let start = u64::try_from(data).unwrap_or(offset);
        let mut at = start;
        while at < end {
            let span = usize::try_from((end - at).min(CHUNK as u64)).unwrap_or(CHUNK);
            source.read_exact_at(&mut buffer[..span], at)?;
            if buffer[..span].iter().any(|byte| *byte != 0) {
                destination.write_all_at(&buffer[..span], at)?;
                written += u64::try_from(span).unwrap_or(0);
            }
            at += u64::try_from(span).unwrap_or(0);
        }
        offset = end;
    }
    destination.set_len(length)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    const MIB: u64 = 1024 * 1024;

    /// Opens a file for the test, readable as well as writable.
    ///
    /// A write-only descriptor fails every read with `EBADF`, which is a property of the fixture
    /// and not of the copy under test, and the test reads both files back.
    fn fixture(path: &std::path::Path, empty: bool) -> File {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .expect("open");
        if empty {
            file.set_len(0).expect("start empty");
        }
        file
    }

    /// A file whose first megabyte is written and whose remaining three are a hole.
    fn sparse_source(path: &std::path::Path) -> File {
        let mut file = fixture(path, true);
        file.write_all(&vec![0x5a_u8; MIB as usize]).expect("write");
        file.set_len(4 * MIB).expect("extend into a hole");
        file
    }

    #[test]
    fn a_sparse_copy_keeps_both_the_bytes_and_the_holes() {
        let scratch = std::env::temp_dir().join(format!("sparse-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("scratch directory");
        let source_path = scratch.join("source.img");
        let destination_path = scratch.join("destination.img");
        let source = sparse_source(&source_path);
        let destination = fixture(&destination_path, true);

        let written = copy_sparse(&source, &destination, 4 * MIB).expect("copy");

        let metadata = destination.metadata().expect("stat the destination");
        assert_eq!(metadata.len(), 4 * MIB, "the length is the class size");
        // Only the written megabyte is not zero, whether the filesystem reported the hole or the
        // walk skipped it by looking at the bytes: either way the copy must not have paid for
        // four megabytes of disk.
        assert_eq!(written, MIB, "one megabyte of the four is data");
        let allocated = metadata.blocks() * 512;
        assert!(
            allocated < 2 * MIB,
            "the copy allocated {allocated} bytes for a {MIB}-byte extent"
        );
        let mut head = vec![0_u8; 16];
        destination
            .read_exact_at(&mut head, 0)
            .expect("read the data");
        assert_eq!(head, vec![0x5a_u8; 16]);
        let mut hole = vec![0xff_u8; 16];
        destination
            .read_exact_at(&mut hole, 3 * MIB)
            .expect("read the hole");
        assert_eq!(hole, vec![0_u8; 16], "the hole reads as zeros");

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
