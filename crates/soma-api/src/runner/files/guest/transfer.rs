//! Moving file contents in transfers the facade will carry.
//!
//! One facade call moves at most [`soma::MAX_FILE_BYTES`], so a file larger than that is read as
//! consecutive ranges and written as consecutive ranges. A write never edits its target in place:
//! it fills a temporary sibling, sets the sibling's mode, and renames it over the target, which
//! is what the platform's guest daemon does too, so a concurrent reader sees the old file or the
//! new one and never a mixture. A write that fails part way removes its sibling.

use soma::{FileAnswer, FileKind, FileOperation, FileRefusal};

<<<<<<< HEAD
use super::{GuestFiles, Stop, ask, kind_of, staged::Staged, stat};
=======
use super::{GuestFiles, Stop, ask, kind_of, make_directory, remove, rename, set_mode, stat};
>>>>>>> origin/r3/soma-unified
use crate::runner::files::path::GuestPath;

/// The most one facade call moves.
const TRANSFER: usize = soma::MAX_FILE_BYTES;
/// The mode a written file ends with when the call names none, as on the platform.
const DEFAULT_FILE_MODE: u32 = 0o644;

/// Which part of a file a ranged read asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Slice {
    /// The last `n` bytes.
    Tail(u64),
    /// `length` bytes from `offset`.
    Range { offset: u64, length: u64 },
}

/// What a ranged read returned: the bytes, where they start, and how big the whole file is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SliceRead {
    pub(crate) bytes: Vec<u8>,
    pub(crate) offset: u64,
    pub(crate) size: u64,
}

/// How a write treats the file already at its path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Written {
    /// Add to the end of an existing file rather than replace it.
    pub(crate) append: bool,
    /// The permission bits a file this write creates gets.
    pub(crate) mode: Option<u32>,
}

/// Reads the whole file, refusing one larger than `cap` bytes as too large.
pub(crate) fn read_whole(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    cap: usize,
) -> Result<Vec<u8>, Stop> {
    let mut contents = Vec::new();
    loop {
        let offset = len(&contents);
        let (bytes, end) = match read_range(guest, path, offset, TRANSFER) {
            Ok(read) => read,
            // A guest without ranged reads still reads a whole file of one transfer.
            Err(Stop::Refused(FileRefusal::Unsupported)) if offset == 0 => {
                match ask(
                    guest,
                    FileOperation::Read {
                        path: bytes_of(path),
                    },
                )? {
                    FileAnswer::Read { bytes } => (bytes, true),
                    _ => return Err(Stop::Unexpected),
                }
            }
            Err(stop) => return Err(stop),
        };
        if contents.len().saturating_add(bytes.len()) > cap {
            return Err(Stop::Refused(FileRefusal::TooLarge));
        }
        let empty = bytes.is_empty();
        contents.extend_from_slice(&bytes);
        if end {
            return Ok(contents);
        }
        // A ranged read that is not at the end returns at least one byte; one that returned
        // none would otherwise be asked again forever.
        if empty {
            return Err(Stop::Unexpected);
        }
    }
}

/// Reads part of a file, with where the part starts and how big the file is.
pub(crate) fn read_slice(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    slice: Slice,
) -> Result<SliceRead, Stop> {
    let size = stat(guest, path)?.size;
    let (start, length) = match slice {
        Slice::Tail(count) => (size.saturating_sub(count), count.min(size)),
        Slice::Range { offset, length } => (offset.min(size), length),
    };
    let wanted = usize::try_from(length).unwrap_or(usize::MAX).min(TRANSFER);
    let bytes = if wanted == 0 || start >= size {
        Vec::new()
    } else {
        read_range(guest, path, start, wanted)?.0
    };
    let offset = match slice {
        Slice::Tail(_) => size.saturating_sub(len(&bytes)),
        Slice::Range { .. } => start,
    };
    Ok(SliceRead {
        bytes,
        offset,
        size,
    })
}

/// Writes `contents` to `path` and returns the size the file has afterwards.
pub(crate) fn write(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    contents: &[u8],
    how: Written,
) -> Result<u64, Stop> {
    let existing = if how.append || how.mode.is_some() {
        kind_of(guest, path)?
    } else {
        None
    };
    if how.append && existing.is_some() {
        let start = stat(guest, path)?.size;
        write_ranges(guest, path, start, contents, false)?;
        return Ok(start.saturating_add(len(contents)));
    }
    // The mode applies to a file this write brings into being, as the platform's contract says;
    // replacing an existing file leaves it at the default, as the platform's guest daemon does.
    let mode = match (existing, how.mode) {
        (None, Some(mode)) => mode,
        _ => DEFAULT_FILE_MODE,
    };
    if existing == Some(FileKind::Directory) {
        return Err(Stop::Refused(FileRefusal::WrongKind));
    }
    replace(guest, path, contents, mode)?;
    Ok(len(contents))
}

/// Fills a temporary sibling, sets its mode, and renames it over `path`.
fn replace(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    contents: &[u8],
    mode: u32,
) -> Result<(), Stop> {
<<<<<<< HEAD
    let mut staged = Staged::begin(guest, path.clone())?;
    let published = staged
        .append(guest, contents)
        .and_then(|()| staged.publish(guest, mode));
    if published.is_err() {
        staged.discard(guest);
    }
    published.map(|_| ())
}

/// Writes `contents` from `start` in transfers, creating the file with the first one when asked.
pub(super) fn write_ranges(
=======
    make_directory(guest, &path.parent())?;
    let temporary = path.sibling(&temporary_name());
    let published = write_ranges(guest, &temporary, 0, contents, true)
        .and_then(|()| set_mode(guest, &temporary, mode))
        .and_then(|()| rename(guest, &temporary, path));
    if published.is_err() {
        // Best effort: the sibling must not outlive the write, and its own removal failing
        // changes nothing about why the write did.
        let _removed = remove(guest, &temporary, false);
    }
    published
}

/// Writes `contents` from `start` in transfers, creating the file with the first one when asked.
fn write_ranges(
>>>>>>> origin/r3/soma-unified
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    start: u64,
    contents: &[u8],
    create: bool,
) -> Result<(), Stop> {
    let mut chunks: Vec<&[u8]> = contents.chunks(TRANSFER).collect();
    if chunks.is_empty() {
        // An empty file is still one write, so it is created.
        chunks.push(&[]);
    }
    let mut offset = start;
    for (index, chunk) in chunks.iter().enumerate() {
        let operation = FileOperation::WriteRange {
            path: bytes_of(path),
            offset,
            create: create && index == 0,
            shorten: false,
            bytes: chunk.to_vec(),
        };
        match ask(guest, operation)? {
            FileAnswer::Written { .. } => offset = offset.saturating_add(len(chunk)),
            _ => return Err(Stop::Unexpected),
        }
    }
    Ok(())
}

/// One ranged read: the bytes and whether they reached the end of the file.
<<<<<<< HEAD
pub(crate) fn read_range(
=======
pub(super) fn read_range(
>>>>>>> origin/r3/soma-unified
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    offset: u64,
    length: usize,
) -> Result<(Vec<u8>, bool), Stop> {
    let length = u32::try_from(length.min(TRANSFER)).map_err(|_| Stop::Unexpected)?;
    match ask(
        guest,
        FileOperation::ReadRange {
            path: bytes_of(path),
            offset,
            length,
        },
    )? {
        FileAnswer::Range { bytes, end } => Ok((bytes, end)),
        _ => Err(Stop::Unexpected),
    }
}

/// A name no caller chose and no two writes share, hidden as the platform's own staging files are.
<<<<<<< HEAD
pub(super) fn temporary_name() -> String {
=======
fn temporary_name() -> String {
>>>>>>> origin/r3/soma-unified
    let unique = uuid::Uuid::new_v4().simple().to_string();
    format!(".soma-write-{}", &unique[..16])
}

fn bytes_of(path: &GuestPath) -> Vec<u8> {
    path.as_bytes().to_vec()
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}
