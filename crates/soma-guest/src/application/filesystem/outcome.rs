//! What the guest answers one filesystem request with.
//!
//! A failure is a small closed set of causes rather than an errno or a message. An errno would
//! leak the guest's implementation to the host, and a message would leak tenant data into
//! whatever recorded it, while a caller only ever needs to know which of these happened.

mod codec;
mod entry;
mod failure;

pub use entry::{DetailedEntry, DirectoryEntry, EntryKind};
pub use failure::FileFailure;

use core::fmt;

/// The guest's answer to one filesystem request.
#[derive(Clone, Eq, PartialEq)]
pub enum FileOutcome {
    /// Bytes read, with whether the file ended within this read.
    Read {
        /// The bytes, which may be shorter than the request asked for.
        bytes: Box<[u8]>,
        /// Whether the read reached the end of the file.
        end: bool,
    },
    /// Bytes written.
    Written {
        /// How many bytes were written.
        bytes: u64,
    },
    /// The directory listing, with whether more entries remain beyond it.
    Listed {
        /// The entries, at most [`super::MAX_ENTRIES`] of them.
        entries: Vec<DirectoryEntry>,
        /// Whether the directory held more entries than this listing carries.
        more: bool,
    },
    /// A directory listing with size, mode, and modification time per entry.
    ListedDetailed {
        /// The entries, at most [`super::MAX_ENTRIES`] of them.
        entries: Vec<DetailedEntry>,
        /// Whether the directory held more entries than this listing carries.
        more: bool,
    },
    /// What the named path is, or that it is absent.
    Status {
        /// The kind, or `None` when nothing exists at the path.
        kind: Option<EntryKind>,
    },
    /// The kind, size, permission bits, and modification time of a path that exists.
    Stat {
        /// What the path is.
        kind: EntryKind,
        /// Size in bytes; zero for anything that is not a regular file.
        size: u64,
        /// Permission bits, within [`super::MAX_FILE_MODE`].
        mode: u32,
        /// Modification time in whole seconds since the Unix epoch, zero when unknown.
        modified: u64,
    },
    /// The operation completed and has nothing to report.
    Done,
    /// The operation did not happen.
    Failed(FileFailure),
}

impl fmt::Debug for FileOutcome {
    /// Reports shapes and never bytes or names.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { bytes, end } => {
                write!(formatter, "Read {{ {} bytes, end: {end} }}", bytes.len())
            }
            Self::Written { bytes } => write!(formatter, "Written {{ {bytes} }}"),
            Self::Listed { entries, more } => {
                write!(
                    formatter,
                    "Listed {{ {} entries, more: {more} }}",
                    entries.len()
                )
            }
            Self::ListedDetailed { entries, more } => write!(
                formatter,
                "ListedDetailed {{ {} entries, more: {more} }}",
                entries.len()
            ),
            Self::Status { kind } => write!(formatter, "Status {{ {kind:?} }}"),
            Self::Stat { kind, size, .. } => write!(formatter, "Stat {{ {kind:?}, {size} bytes }}"),
            Self::Done => formatter.write_str("Done"),
            Self::Failed(failure) => write!(formatter, "Failed({failure})"),
        }
    }
}
