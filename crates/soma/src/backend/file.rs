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

/// Largest guest path this protocol will carry.
///
/// The value belongs to the guest protocol, which checks it again on the way in and is the
/// authority on it. It is restated here because this crate is below the protocol crate and a
/// surface has to be able to refuse an inadmissible path before it reaches the wire. The two are
/// held equal by a test in `soma-local`, which depends on both.
pub const MAX_GUEST_PATH_BYTES: usize = 4096;

/// Why a path cannot be carried to a guest at all.
///
/// This is not a filesystem outcome. A path of this shape is one the protocol will not encode, so
/// it never becomes an operation the guest declines; it is refused where the request is built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathRejected {
    /// The path is empty.
    Empty,
    /// The path is longer than [`MAX_GUEST_PATH_BYTES`].
    TooLong,
    /// The path does not begin at the guest's root.
    Relative,
    /// The path carries a nul byte, which no guest path may.
    InteriorNul,
}

impl PathRejected {
    /// A sentence naming what is wrong, for a surface to report.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "the path is empty",
            Self::TooLong => "the path is longer than the guest protocol carries",
            Self::Relative => "the path must be absolute, beginning at the guest's root",
            Self::InteriorNul => "the path carries a nul byte",
        }
    }
}

/// Refuses a path the guest protocol will not carry.
///
/// A request built with one of these would not reach the guest as a filesystem request at all:
/// the guest rejects it while decoding, which is a protocol fault and ends the session. So a
/// caller that named such a path would destroy its own sandbox instead of being told no. The
/// guest still performs this check itself; this one exists so the answer is a refusal.
///
/// What is admissible beyond this shape is the guest's decision and is taken inside the guest.
/// Nothing here resolves, normalises, or approves a path.
///
/// # Errors
///
/// Returns the shape rule the path breaks.
pub const fn check_guest_path(path: &[u8]) -> Result<(), PathRejected> {
    if path.is_empty() {
        return Err(PathRejected::Empty);
    }
    if path.len() > MAX_GUEST_PATH_BYTES {
        return Err(PathRejected::TooLong);
    }
    if path[0] != b'/' {
        return Err(PathRejected::Relative);
    }
    let mut index = 0;
    while index < path.len() {
        if path[index] == 0 {
            return Err(PathRejected::InteriorNul);
        }
        index += 1;
    }
    Ok(())
}

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
