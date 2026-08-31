//! One bounded filesystem operation against a live Instance.
//!
//! The six operations here are the ones the provider contract names, and each is one call: a
//! caller reads a file, writes a file, makes a directory, lists a directory, asks whether a path
//! exists, or removes a path. Nothing here is a filesystem abstraction. It is the portable shape
//! of a request that a backend forwards to the guest that will actually perform it.
//!
//! Paths are bytes rather than text, because a guest path is not required to be UTF-8, and file
//! contents are bytes for the same reason: a surface that carried them as strings would corrupt
//! every file that is not valid UTF-8, quietly.
//!
//! Whether a path is admissible is the guest's decision and is taken inside the guest. This
//! facade bounds only what it must bound to keep one call one bounded unit of work: how many
//! bytes of a file one call will hold in host memory.

mod answer;

pub use answer::{FileAnswer, FileEntry, FileKind, FileObservation, FileRefusal};

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{InstanceId, OperationId};

/// Largest number of bytes one read or write through this facade will move.
///
/// A guest file can be any size the sandbox grew it to, so a call that named a file without
/// bounding it would let the guest decide how much host memory the call takes. The bound is
/// stated here, once, rather than left to each surface to remember.
/// Four mebibytes is the working figure. A hosted machine relays this operation to the process
/// holding it as one bounded JSON line, where a byte becomes up to four characters, so the bound
/// has to leave the relayed form comfortably inside the line ceiling that path already enforces.
pub const MAX_FILE_BYTES: usize = 4 << 20;

/// What a caller asks of one Instance's filesystem.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    /// Reads the whole file at `path`, for at most [`MAX_FILE_BYTES`].
    Read {
        /// Absolute guest path.
        path: Vec<u8>,
    },
    /// Replaces the file at `path` with exactly `bytes`, creating it when absent.
    Write {
        /// Absolute guest path.
        path: Vec<u8>,
        /// The exact contents the file ends up with.
        bytes: Vec<u8>,
    },
    /// Creates the directory at `path` and any missing parent of it.
    MakeDirectory {
        /// Absolute guest path.
        path: Vec<u8>,
    },
    /// Lists the directory at `path`.
    ReadDirectory {
        /// Absolute guest path.
        path: Vec<u8>,
    },
    /// Reports whether `path` exists and what it is.
    Exists {
        /// Absolute guest path.
        path: Vec<u8>,
    },
    /// Removes `path`, and its contents when `recursive` is set.
    Remove {
        /// Absolute guest path.
        path: Vec<u8>,
        /// Whether a directory is removed with its contents.
        recursive: bool,
    },
}

impl FileOperation {
    /// The path this operation names.
    #[must_use]
    pub fn path(&self) -> &[u8] {
        match self {
            Self::Read { path }
            | Self::Write { path, .. }
            | Self::MakeDirectory { path }
            | Self::ReadDirectory { path }
            | Self::Exists { path }
            | Self::Remove { path, .. } => path,
        }
    }

    /// The operation's own name, as every surface reports it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Read { .. } => "read",
            Self::Write { .. } => "write",
            Self::MakeDirectory { .. } => "mkdir",
            Self::ReadDirectory { .. } => "list",
            Self::Exists { .. } => "exists",
            Self::Remove { .. } => "remove",
        }
    }
}

impl fmt::Debug for FileOperation {
    /// Names the operation and the sizes, and never the path or the bytes.
    ///
    /// A guest path and the contents of a guest file are tenant data, so neither may reach a log
    /// through a derived formatter. This mirrors the guest protocol's own redaction, which would
    /// otherwise be undone the moment a request was rebuilt on this side of the seam.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "FileOperation::{} {{ path: {} bytes",
            self.name(),
            self.path().len()
        )?;
        match self {
            Self::Write { bytes, .. } => write!(formatter, ", contents: {} bytes }}", bytes.len()),
            Self::Remove { recursive, .. } => write!(formatter, ", recursive: {recursive} }}"),
            _ => formatter.write_str(" }"),
        }
    }
}

/// One filesystem operation addressed to one exact Instance.
#[derive(Clone, Copy, Debug)]
pub struct FileRequest<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    operation: &'a FileOperation,
}

impl<'a> FileRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        operation: &'a FileOperation,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            operation,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn operation(&self) -> &FileOperation {
        self.operation
    }
}
