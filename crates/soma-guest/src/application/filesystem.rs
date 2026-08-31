//! Bounded filesystem requests carried by one authenticated record each.
//!
//! A sandbox that can only run a command is not one an agent can work in: reading a file it
//! wrote, writing one for the next step, and listing what exists are the operations the provider
//! contract names, and none of them existed on this protocol.
//!
//! One record carries at most [`MAX_BODY_SIZE`] bytes, so a request that names a whole file
//! cannot be a request that carries one. Every operation here is therefore addressed by an
//! explicit offset and a bounded length, and a caller moves a large file by issuing several
//! requests. That keeps one record one bounded unit of work, which is what the record layer
//! already guarantees, instead of inventing a second framing layer inside it.
//!
//! Paths are bytes rather than text, because a guest path is not required to be UTF-8. They are
//! bounded, and rejected when empty, relative, or carrying an interior nul. Resolving one, and
//! refusing whatever the guest's own policy refuses, belongs to the agent that executes it.

mod codec;
mod outcome;

#[cfg(test)]
mod tests;

pub use outcome::{DirectoryEntry, EntryKind, FileFailure, FileOutcome};

use core::fmt;

use crate::Error;

use super::{MAX_BODY_SIZE, frame::Reader};

/// Largest path this protocol will carry.
pub const MAX_PATH_BYTES: usize = 4096;
/// Largest number of bytes one read or write request may move.
///
/// One record holds a bounded body, and a request also carries its path and its fixed fields, so
/// the payload allowance is what remains after the largest path this protocol admits.
pub const MAX_CHUNK_BYTES: usize = {
    let remaining = MAX_BODY_SIZE - MAX_PATH_BYTES - 32;
    // Every field on this protocol carries a `u16` length, so a chunk can never exceed what
    // that prefix can describe however much record body happens to be left.
    if remaining > u16::MAX as usize {
        u16::MAX as usize
    } else {
        remaining
    }
};
/// Largest number of directory entries one listing will return.
pub const MAX_ENTRIES: usize = 1024;

/// What a caller asks of the guest's filesystem.
#[derive(Clone, Eq, PartialEq)]
pub enum FileRequest {
    /// Reads at most `length` bytes from `offset`.
    Read {
        /// Absolute guest path.
        path: Box<[u8]>,
        /// Byte offset to read from.
        offset: u64,
        /// Most bytes to return.
        length: u32,
    },
    /// Writes `bytes` at `offset`, creating the file when `create` is set.
    Write {
        /// Absolute guest path.
        path: Box<[u8]>,
        /// Byte offset to write at.
        offset: u64,
        /// Whether a missing file is created.
        create: bool,
        /// Whether the file ends where this write ends.
        shorten: bool,
        /// The bytes to write.
        bytes: Box<[u8]>,
    },
    /// Creates a directory, and its missing parents when `parents` is set.
    MakeDirectory {
        /// Absolute guest path.
        path: Box<[u8]>,
        /// Whether missing parents are created.
        parents: bool,
    },
    /// Lists at most [`MAX_ENTRIES`] entries, skipping the first `offset`.
    ReadDirectory {
        /// Absolute guest path.
        path: Box<[u8]>,
        /// Entries to skip, so a large directory is read across several requests.
        offset: u32,
    },
    /// Reports whether a path exists and what it is.
    Exists {
        /// Absolute guest path.
        path: Box<[u8]>,
    },
    /// Removes a file, or a directory with its contents when `recursive` is set.
    Remove {
        /// Absolute guest path.
        path: Box<[u8]>,
        /// Whether a directory is removed with its contents.
        recursive: bool,
    },
}

impl FileRequest {
    /// The path this request names.
    #[must_use]
    pub fn path(&self) -> &[u8] {
        match self {
            Self::Read { path, .. }
            | Self::Write { path, .. }
            | Self::MakeDirectory { path, .. }
            | Self::ReadDirectory { path, .. }
            | Self::Exists { path }
            | Self::Remove { path, .. } => path,
        }
    }
}

impl fmt::Debug for FileRequest {
    /// Names the operation and the path length, and never the path or the bytes.
    ///
    /// A guest path and the contents of a guest file are tenant data, so neither may reach a log
    /// through a derived formatter.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self {
            Self::Read { .. } => "read",
            Self::Write { .. } => "write",
            Self::MakeDirectory { .. } => "make-directory",
            Self::ReadDirectory { .. } => "read-directory",
            Self::Exists { .. } => "exists",
            Self::Remove { .. } => "remove",
        };
        write!(
            formatter,
            "FileRequest::{operation} {{ path: {} bytes }}",
            self.path().len()
        )
    }
}

/// Rejects a path this protocol will not carry.
///
/// A path is bounded, non-empty, absolute, and free of interior nul bytes. Everything else about
/// it, including whether the guest's policy admits it at all, is the executing agent's decision.
pub(super) fn check_path(path: &[u8]) -> Result<(), Error> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(Error::ApplicationMessageRejected);
    }
    if path[0] != b'/' || path.contains(&0) {
        return Err(Error::ApplicationMessageRejected);
    }
    Ok(())
}

/// Reads one bounded path field and rejects one this protocol will not carry.
pub(super) fn read_path<'a>(reader: &mut Reader<'a>) -> Result<&'a [u8], Error> {
    let path = reader.field(MAX_PATH_BYTES)?;
    check_path(path)?;
    Ok(path)
}
