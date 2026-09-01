//! Translation between portable filesystem operations and the guest protocol.

use soma::{FileAnswer, FileEntry, FileKind, FileOperation, FileRefusal};
use soma_guest::{
    DirectoryEntry, EntryKind, FileFailure, FileOutcome, FileRequest as GuestFileRequest,
    WholeFileRead, WholeFileWrite,
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
        FileOperation::Read { .. } | FileOperation::Write { .. } => None,
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
