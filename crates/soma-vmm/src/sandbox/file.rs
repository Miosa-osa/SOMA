//! Translation between portable filesystem operations and the guest protocol.

use soma::{FileAnswer, FileEntry, FileKind, FileOperation, FileRefusal};
use soma_guest::{
    DirectoryEntry, EntryKind, FileFailure, FileOutcome, FileRequest as GuestFileRequest,
    RangeRead, WholeFileRead, WholeFileWrite,
};

pub(super) fn answer_from(outcome: FileOutcome) -> FileAnswer {
    match outcome {
        FileOutcome::Read { bytes, .. } => FileAnswer::Read {
            bytes: bytes.into_vec(),
        },
        FileOutcome::Written { bytes } => FileAnswer::Written { bytes },
        FileOutcome::Listed { entries, more } => FileAnswer::Listed {
            entries: entries.into_iter().map(entry_from).collect(),
            more,
        },
        FileOutcome::Status { kind } => FileAnswer::Status {
            kind: kind.map(kind_from),
        },
        FileOutcome::Stat {
            kind,
            size,
            mode,
            modified,
        } => FileAnswer::Stat {
            kind: kind_from(kind),
            size,
            mode,
            modified,
        },
        FileOutcome::Done => FileAnswer::Done,
        FileOutcome::Failed(failure) => FileAnswer::Refused(refusal_from(failure)),
    }
}

const fn refusal_from(failure: FileFailure) -> FileRefusal {
    match failure {
        FileFailure::NotFound => FileRefusal::NotFound,
        FileFailure::Denied => FileRefusal::Denied,
        FileFailure::WrongKind => FileRefusal::WrongKind,
        FileFailure::Exists => FileRefusal::Exists,
        FileFailure::NotEmpty => FileRefusal::NotEmpty,
        FileFailure::Failed => FileRefusal::Failed,
    }
}

const fn kind_from(kind: EntryKind) -> FileKind {
    match kind {
        EntryKind::File => FileKind::File,
        EntryKind::Directory => FileKind::Directory,
        EntryKind::Other => FileKind::Other,
    }
}

fn entry_from(entry: DirectoryEntry) -> FileEntry {
    FileEntry {
        name: entry.name.into_vec(),
        kind: kind_from(entry.kind),
    }
}

pub(super) fn single_request(operation: &FileOperation) -> Option<GuestFileRequest> {
    match operation {
        FileOperation::MakeDirectory { path } => Some(GuestFileRequest::MakeDirectory {
            path: path.as_slice().into(),
            parents: true,
        }),
        FileOperation::ReadDirectory { path } => Some(GuestFileRequest::ReadDirectory {
            path: path.as_slice().into(),
            offset: 0,
        }),
        FileOperation::Exists { path } => Some(GuestFileRequest::Exists {
            path: path.as_slice().into(),
        }),
        FileOperation::Remove { path, recursive } => Some(GuestFileRequest::Remove {
            path: path.as_slice().into(),
            recursive: *recursive,
        }),
        FileOperation::Stat { path } => Some(GuestFileRequest::Stat {
            path: path.as_slice().into(),
        }),
        FileOperation::Rename { path, to } => Some(GuestFileRequest::Rename {
            path: path.as_slice().into(),
            to: to.as_slice().into(),
        }),
        FileOperation::SetMode { path, mode } => Some(GuestFileRequest::SetMode {
            path: path.as_slice().into(),
            mode: *mode,
        }),
        FileOperation::Read { .. }
        | FileOperation::Write { .. }
        | FileOperation::ReadRange { .. }
        | FileOperation::WriteRange { .. } => None,
    }
}

/// Whether the guest must speak API level 2 to answer this operation.
pub(super) const fn needs_ranged_files(operation: &FileOperation) -> bool {
    matches!(
        operation,
        FileOperation::Stat { .. }
            | FileOperation::Rename { .. }
            | FileOperation::SetMode { .. }
            | FileOperation::ReadRange { .. }
            | FileOperation::WriteRange { .. }
    )
}

/// Refuses an operation the guest protocol would reject while decoding.
///
/// A guest rejecting a malformed body ends the session, so the shape rules are checked here and
/// the caller gets a refusal instead of losing its sandbox.
pub(super) fn admissible(operation: &FileOperation) -> bool {
    let paths = std::iter::once(operation.path()).chain(match operation {
        FileOperation::Rename { to, .. } => Some(to.as_slice()),
        _ => None,
    });
    for path in paths {
        if soma::check_guest_path(path).is_err() {
            return false;
        }
    }
    match operation {
        FileOperation::SetMode { mode, .. } => *mode <= soma_guest::MAX_FILE_MODE,
        FileOperation::ReadRange { length, .. } => {
            usize::try_from(*length).is_ok_and(|length| length <= soma::MAX_FILE_BYTES)
        }
        _ => true,
    }
}

pub(super) fn range_answer(read: RangeRead) -> FileAnswer {
    match read {
        RangeRead::Bytes { bytes, end } => FileAnswer::Range { bytes, end },
        RangeRead::TooLarge => FileAnswer::Refused(FileRefusal::TooLarge),
        RangeRead::Failed(failure) => FileAnswer::Refused(refusal_from(failure)),
    }
}

pub(super) fn read_answer(read: WholeFileRead) -> FileAnswer {
    match read {
        WholeFileRead::Bytes(bytes) => FileAnswer::Read { bytes },
        WholeFileRead::TooLarge => FileAnswer::Refused(FileRefusal::TooLarge),
        WholeFileRead::Failed(failure) => FileAnswer::Refused(refusal_from(failure)),
    }
}

pub(super) fn write_answer(outcome: WholeFileWrite, length: usize) -> FileAnswer {
    match outcome {
        WholeFileWrite::Written => FileAnswer::Written {
            bytes: u64::try_from(length).unwrap_or(u64::MAX),
        },
        WholeFileWrite::TooLarge => FileAnswer::Refused(FileRefusal::TooLarge),
        WholeFileWrite::Failed(failure) => FileAnswer::Refused(refusal_from(failure)),
    }
}

#[cfg(test)]
mod tests {
    use soma::FileOperation;

    use super::{admissible, needs_ranged_files};

    fn path() -> Vec<u8> {
        b"/f".to_vec()
    }

    #[test]
    fn only_level_two_operations_need_the_probe() {
        assert!(!needs_ranged_files(&FileOperation::Read { path: path() }));
        assert!(!needs_ranged_files(&FileOperation::Remove {
            path: path(),
            recursive: true
        }));
        for operation in [
            FileOperation::Stat { path: path() },
            FileOperation::Rename {
                path: path(),
                to: path(),
            },
            FileOperation::SetMode {
                path: path(),
                mode: 0o600,
            },
            FileOperation::ReadRange {
                path: path(),
                offset: 0,
                length: 1,
            },
            FileOperation::WriteRange {
                path: path(),
                offset: 0,
                create: true,
                shorten: false,
                bytes: Vec::new(),
            },
        ] {
            assert!(needs_ranged_files(&operation), "{operation:?}");
        }
    }

    #[test]
    fn a_request_the_guest_would_reject_is_not_admissible() {
        let rename = |to: &[u8]| FileOperation::Rename {
            path: path(),
            to: to.to_vec(),
        };
        assert!(admissible(&rename(b"/g")));
        assert!(!admissible(&rename(b"relative")));
        assert!(!admissible(&rename(b"")));
        assert!(!admissible(&rename(b"/a\0b")));
        assert!(!admissible(&FileOperation::SetMode {
            path: path(),
            mode: 0o4755
        }));
        assert!(admissible(&FileOperation::SetMode {
            path: path(),
            mode: 0o777
        }));
        let range = |length: u32| FileOperation::ReadRange {
            path: path(),
            offset: 0,
            length,
        };
        assert!(admissible(&range(
            u32::try_from(soma::MAX_FILE_BYTES).unwrap()
        )));
        assert!(!admissible(&range(u32::MAX)));
        assert!(!admissible(&FileOperation::Stat {
            path: vec![b'/'; soma::MAX_GUEST_PATH_BYTES + 1]
        }));
    }
}
