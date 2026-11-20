//! Performing one checked file call against a guest, and the answer it produces.

use soma::{FileKind, FileRefusal};

use super::{
    FileCall,
    answer::{
        Contents, Data, Deleted, ListedEntry, Listing, ModeSet, Moved, StatData, Status,
        WrittenFields, WrittenFile,
    },
    error::FileError,
    guest::{self, GuestFiles, Stop},
    mode::Mode,
    path::GuestPath,
    plan::{MAX_JSON_FILE_BYTES, MAX_JSON_FILE_LABEL, Plan},
};
use crate::runner::{clock::iso8601_seconds, public_wire::encode};

/// One finished file call: the status and the body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

impl Answer {
    fn json<T: serde::Serialize>(status: u16, body: &T) -> Self {
        Self {
            status,
            body: encode(body),
        }
    }
}

impl Plan {
    /// Performs the call.
    ///
    /// # Errors
    ///
    /// Returns why the guest did not complete it.
    pub(crate) fn run(self, guest: &mut dyn GuestFiles) -> Result<Answer, Stop> {
        match self {
            Self::List { echo, path } => {
                let (entries, more) = guest::list(guest, &path)?;
                let files = entries.iter().filter_map(ListedEntry::of).collect();
                let listing = Listing {
                    files,
                    path: &echo,
                    truncated: more,
                };
                Ok(Answer::json(200, &listing))
            }
            Self::Read { echo, path } => {
                let bytes = guest::read_whole(guest, &path, MAX_JSON_FILE_BYTES)?;
                Ok(Answer::json(200, &Contents::whole(bytes, &echo)))
            }
            Self::ReadSlice { echo, path, slice } => {
                let read = guest::read_slice(guest, &path, slice)?;
                let part = Contents::part(read.bytes, &echo, read.offset, read.size);
                Ok(Answer::json(200, &part))
            }
            Self::Write {
                echo,
                path,
                contents,
                how,
            } => {
                let size_bytes = guest::write(guest, &path, &contents, how)?;
                let file = WrittenFields {
                    filename: path.file_name(),
                    path: &echo,
                    size_bytes,
                };
                Ok(Answer::json(
                    201,
                    &WrittenFile {
                        file,
                        success: true,
                    },
                ))
            }
            Self::Stat { echo, path } => stat(guest, &echo, &path),
            Self::MakeDirectory { path } => {
                guest::make_directory(guest, &path)?;
                Ok(Answer::json(
                    201,
                    &Data {
                        data: Status { status: "ok" },
                    },
                ))
            }
            Self::Rename { from, to } => {
                guest::move_to(guest, &from, &to)?;
                let data = Moved {
                    from: from.as_str(),
                    to: to.as_str(),
                };
                Ok(Answer::json(200, &Data { data }))
            }
            Self::Delete {
                echo,
                path,
                recursive,
            } => {
                guest::remove(guest, &path, recursive)?;
                let deleted = Deleted {
                    path: &echo,
                    success: true,
                };
                Ok(Answer::json(200, &deleted))
            }
            Self::Chmod { path, mode, echo } => chmod(guest, &path, &mode, &echo),
        }
    }
}

fn stat(guest: &mut dyn GuestFiles, echo: &str, path: &GuestPath) -> Result<Answer, Stop> {
    let stat = guest::stat(guest, path)?;
    let data = StatData {
        is_dir: stat.kind == FileKind::Directory,
        is_file: stat.kind == FileKind::File,
        is_symlink: false,
        mode: format!("0o{:o}", stat.mode),
        modified_at: iso8601_seconds(stat.modified),
        name: path.file_name(),
        path: echo,
        size: stat.size,
        size_bytes: stat.size,
    };
    Ok(Answer::json(200, &Data { data }))
}

/// A symbolic mode is applied to the path's current bits, so it costs one stat first.
fn chmod(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    mode: &Mode,
    echo: &str,
) -> Result<Answer, Stop> {
    let bits = if mode.is_relative() {
        let stat = guest::stat(guest, path)?;
        mode.apply(stat.mode, stat.kind == FileKind::Directory)
    } else {
        mode.apply(0, false)
    };
    guest::set_mode(guest, path, bits)?;
    let data = ModeSet {
        mode: echo,
        path: path.as_str(),
    };
    Ok(Answer::json(200, &Data { data }))
}

/// The refusal one guest cause becomes for one call.
///
/// A read refused as too large is told which route moves a file of any size; every other cause
/// is answered the same way for every call.
pub(crate) fn refusal(call: FileCall, refusal: FileRefusal) -> FileError {
    match (call, refusal) {
        (FileCall::Read, FileRefusal::TooLarge) => {
            FileError::too_large(MAX_JSON_FILE_LABEL, "fs/download")
        }
        (_, refusal) => FileError::refused(call.name(), refusal),
    }
}
