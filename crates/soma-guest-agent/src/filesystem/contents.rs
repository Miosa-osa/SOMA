//! Reading and writing one bounded run of bytes.
//!
//! One record carries a bounded body, so neither operation moves a whole file. Each names an
//! explicit offset, and a caller moves a large file by issuing several requests. That is why a
//! read reports whether it reached the end: it is the only way the caller knows to stop asking.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use soma_guest::{FileFailure, FileOutcome, MAX_CHUNK_BYTES};

use super::failure;

/// Reads at most `length` bytes from `offset`, never more than one record can carry.
///
/// A request built inside the guest could ask for more than the outcome can encode, so the
/// length is capped rather than refused: a short read is already part of this contract, and the
/// caller learns the file has not ended and asks again.
pub(super) fn read(path: &Path, offset: u64, length: u32) -> FileOutcome {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => return failure::failed(&error),
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => return failure::failed(&error),
    };
    // Opening a directory succeeds and only the read fails, so the kind is checked here to name
    // the cause instead of letting the read report it as an unclassified failure.
    if !metadata.is_file() {
        return FileOutcome::Failed(FileFailure::WrongKind);
    }
    if let Err(error) = file.seek(SeekFrom::Start(offset)) {
        return failure::failed(&error);
    }
    let wanted = usize::try_from(length)
        .unwrap_or(usize::MAX)
        .min(MAX_CHUNK_BYTES);
    let mut bytes = vec![0_u8; wanted];
    let filled = match fill(&mut file, &mut bytes) {
        Ok(filled) => filled,
        Err(error) => return failure::failed(&error),
    };
    bytes.resize(filled, 0);
    let reached = offset.saturating_add(u64::try_from(filled).unwrap_or(u64::MAX));
    FileOutcome::Read {
        bytes: bytes.into_boxed_slice(),
        end: reached >= metadata.len(),
    }
}

/// Writes `bytes` at `offset`, and ends the file there when the request asks it to.
///
/// Shortening is part of the same request rather than a separate one because a caller replacing
/// a file writes its last chunk and its new length together; splitting them would leave the old
/// tail visible between the two.
pub(super) fn write(
    path: &Path,
    offset: u64,
    create: bool,
    shorten: bool,
    bytes: &[u8],
) -> FileOutcome {
    let mut file = match OpenOptions::new().write(true).create(create).open(path) {
        Ok(file) => file,
        Err(error) => return failure::failed(&error),
    };
    if let Err(error) = file.seek(SeekFrom::Start(offset)) {
        return failure::failed(&error);
    }
    if let Err(error) = file.write_all(bytes) {
        return failure::failed(&error);
    }
    let written = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if shorten && let Err(error) = file.set_len(offset.saturating_add(written)) {
        return failure::failed(&error);
    }
    FileOutcome::Written { bytes: written }
}

/// Fills as much of the buffer as the file has, returning how many bytes arrived.
///
/// A single `read` may return less than the file holds for reasons that are not the end of it,
/// so a short read is retried; only a zero-length one means the file ended.
fn fill(file: &mut File, bytes: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < bytes.len() {
        match file.read(&mut bytes[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}
