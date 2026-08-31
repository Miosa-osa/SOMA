//! Creating a directory and listing one.
//!
//! A listing is bounded like every other operation, so a large directory is read across several
//! requests: the caller skips what it has already seen and is told whether anything remains.
//! The order is whatever the kernel returns, which is stable for an unchanging directory and is
//! the only order a paged listing can offer without holding the whole of it in memory.

use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use soma_guest::{
    DirectoryEntry, EntryKind, FileFailure, FileOutcome, MAX_ENTRIES, MAX_PATH_BYTES,
};

use super::{entry, failure};

/// Creates the directory, and its missing parents when the request asks for them.
///
/// Asking for parents also makes an existing directory succeed, because that is what the request
/// means: the caller wants the directory to be there, not to have been the one who made it.
pub(super) fn make(path: &Path, parents: bool) -> FileOutcome {
    let made = if parents {
        fs::create_dir_all(path)
    } else {
        fs::create_dir(path)
    };
    failure::done(made)
}

/// Lists at most [`MAX_ENTRIES`] entries after skipping the first `offset` of them.
pub(super) fn list(path: &Path, offset: u32) -> FileOutcome {
    let reader = match fs::read_dir(path) {
        Ok(reader) => reader,
        Err(error) => return failure::failed(&error),
    };
    let mut entries = Vec::new();
    let mut more = false;
    for item in reader.skip(usize::try_from(offset).unwrap_or(usize::MAX)) {
        let item = match item {
            Ok(item) => item,
            Err(error) => return failure::failed(&error),
        };
        if entries.len() == MAX_ENTRIES {
            more = true;
            break;
        }
        // A kind that cannot be read means the entry was removed while the listing ran. Reporting
        // it as the kind the protocol keeps for everything else costs the caller one wasted
        // lookup, where failing the whole listing would cost it the entire directory.
        let kind = item.file_type().map_or(EntryKind::Other, entry::kind_of);
        let name = item.file_name().into_vec();
        if name.is_empty() || name.len() > MAX_PATH_BYTES {
            return FileOutcome::Failed(FileFailure::Failed);
        }
        entries.push(DirectoryEntry {
            name: name.into_boxed_slice(),
            kind,
        });
    }
    FileOutcome::Listed { entries, more }
}
