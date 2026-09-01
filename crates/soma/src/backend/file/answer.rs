//! What one filesystem operation answered with.
//!
//! A refusal is a small closed set of causes rather than an errno or a message, for the reason
//! the guest protocol gives: an errno leaks the guest's implementation to the host, and a message
//! leaks tenant data into whatever recorded it. The set here is the guest's own set, carried
//! across unchanged, plus one cause the guest cannot produce because this side enforces it.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{InstanceId, OperationId};

/// Why a filesystem operation did not happen.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileRefusal {
    /// No such path.
    NotFound,
    /// The guest refused the access.
    Denied,
    /// A directory was named where a file was required, or the reverse.
    WrongKind,
    /// The path already exists and the operation required that it did not.
    Exists,
    /// A directory was not empty and the operation did not ask to remove its contents.
    NotEmpty,
    /// The file is larger than one call through this facade will hold.
    ///
    /// This is the one cause the guest never reports. It belongs to the bound this side puts on
    /// a transfer, stated by [`super::MAX_FILE_BYTES`], and is a refusal rather than a failure:
    /// nothing about the guest or the transport went wrong, and the sandbox stays usable.
    TooLarge,
    /// The guest could not complete the operation for any other reason.
    Failed,
}

impl fmt::Display for FileRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl FileRefusal {
    /// The stable name a surface reports this refusal by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Denied => "denied",
            Self::WrongKind => "wrong_kind",
            Self::Exists => "already_exists",
            Self::NotEmpty => "not_empty",
            Self::TooLarge => "too_large",
            Self::Failed => "failed",
        }
    }
}

/// What one path is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// Anything else the guest reports, including a symbolic link or a device.
    Other,
}

impl FileKind {
    /// The stable name a surface reports this kind by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Other => "other",
        }
    }
}

/// One directory entry: its own name within the directory, never a path.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct FileEntry {
    /// The entry's own name, with no separator in it.
    pub name: Vec<u8>,
    /// What the entry is.
    pub kind: FileKind,
}

impl fmt::Debug for FileEntry {
    /// Names the kind and the name length, never the name, which is tenant data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "FileEntry {{ kind: {:?}, name: {} bytes }}",
            self.kind,
            self.name.len()
        )
    }
}

/// The answer to one filesystem operation.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAnswer {
    /// The whole contents of the file that was read.
    Read {
        /// The bytes, exactly as the guest holds them.
        bytes: Vec<u8>,
    },
    /// The file was written.
    Written {
        /// How many bytes the file now holds.
        bytes: u64,
    },
    /// The directory listing.
    Listed {
        /// The entries.
        entries: Vec<FileEntry>,
        /// Whether the directory held more entries than this listing carries.
        more: bool,
    },
    /// What the named path is, or that it is absent.
    Status {
        /// The kind, or `None` when nothing exists at the path.
        kind: Option<FileKind>,
    },
    /// The operation completed and has nothing to report.
    Done,
    /// The operation did not happen.
    Refused(FileRefusal),
}

impl fmt::Debug for FileAnswer {
    /// Reports shapes and never bytes or names, for the reason [`FileEntry`] gives.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { bytes } => write!(formatter, "Read {{ {} bytes }}", bytes.len()),
            Self::Written { bytes } => write!(formatter, "Written {{ {bytes} }}"),
            Self::Listed { entries, more } => {
                write!(
                    formatter,
                    "Listed {{ {} entries, more: {more} }}",
                    entries.len()
                )
            }
            Self::Status { kind } => write!(formatter, "Status {{ {kind:?} }}"),
            Self::Done => formatter.write_str("Done"),
            Self::Refused(refusal) => write!(formatter, "Refused({refusal})"),
        }
    }
}

/// One filesystem answer, bound to the operation and Instance it belongs to.
///
/// The identities travel with the answer so the engine can refuse an answer that names a
/// different operation or a different Instance than the one it asked about, rather than
/// reporting one sandbox's filesystem as another's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservation {
    operation_id: OperationId,
    instance_id: InstanceId,
    answer: FileAnswer,
}

impl FileObservation {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        answer: FileAnswer,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            answer,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn answer(&self) -> &FileAnswer {
        &self.answer
    }

    pub(crate) fn into_answer(self) -> FileAnswer {
        self.answer
    }
}
