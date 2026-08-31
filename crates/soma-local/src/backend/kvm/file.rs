//! One portable filesystem operation, carried to the guest that performs it.
//!
//! The translation between the portable operation and the guest protocol lives here and nowhere
//! else. It is a mapping, not a layer: each of the six operations becomes one guest request, or,
//! for the two that move a whole file, the bounded chunk loop the guest session already exposes.
//!
//! A path is not resolved, normalised, or rewritten on the way through. The guest validates every
//! path it is given, refuses what its own policy refuses, and is the only side that knows what
//! the sandbox's filesystem actually contains, so a host that pre-approved a path would be
//! deciding something it cannot see.

use soma::{
    BackendFailure, BackendFailureKind, FileAnswer, FileEntry, FileKind, FileObservation,
    FileOperation, FileRefusal, FileRequest, InstanceId,
};
use soma_guest::{
    DirectoryEntry, EntryKind, FileFailure, FileOutcome, FileRequest as GuestFileRequest,
    WholeFileRead, WholeFileWrite,
};

use super::{KvmBackend, host, start::failure_kind};

impl KvmBackend {
    pub(in crate::backend) fn file(
        &mut self,
        request: FileRequest<'_>,
    ) -> Result<FileObservation, BackendFailure> {
        let operation = request.operation_id();
        let instance = request.instance_id().clone();
        let answer = match self.hosted_directory() {
            None => self.file_resident(&instance, request.operation()),
            Some(directory) => host::file(&directory, &instance, request.operation())
                .map_err(|failure| self.host_kind(failure, &instance)),
        };
        let answer = answer.map_err(|kind| self.fail(operation, kind))?;
        Ok(FileObservation::new(operation.clone(), instance, answer))
    }
}

/// What one guest answer becomes on the portable side.
///
/// The guest's closed failure set is carried across one to one. Nothing widens it: a cause the
/// host invented would tell a caller the guest said something it did not.
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

pub(super) const fn refusal_from(failure: FileFailure) -> FileRefusal {
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

/// The four operations that are exactly one guest request each.
///
/// Read and write are not here: each moves a whole file, which the record layer bounds to one
/// chunk, so they take the session's own chunk loop instead.
pub(super) fn single_request(operation: &FileOperation) -> Option<GuestFileRequest> {
    match operation {
        FileOperation::MakeDirectory { path } => Some(GuestFileRequest::MakeDirectory {
            path: path.as_slice().into(),
            // A caller asking for a directory means the directory to exist, and a parent it did
            // not name is not a decision it was trying to make.
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

/// What a bounded whole-file read answered.
pub(super) fn read_answer(read: WholeFileRead) -> FileAnswer {
    match read {
        WholeFileRead::Bytes(bytes) => FileAnswer::Read { bytes },
        WholeFileRead::TooLarge => FileAnswer::Refused(FileRefusal::TooLarge),
        WholeFileRead::Failed(failure) => FileAnswer::Refused(refusal_from(failure)),
    }
}

/// What a bounded whole-file write answered.
pub(super) fn write_answer(outcome: WholeFileWrite, length: usize) -> FileAnswer {
    match outcome {
        WholeFileWrite::Written => FileAnswer::Written {
            bytes: u64::try_from(length).unwrap_or(u64::MAX),
        },
        WholeFileWrite::TooLarge => FileAnswer::Refused(FileRefusal::TooLarge),
        WholeFileWrite::Failed(failure) => FileAnswer::Refused(refusal_from(failure)),
    }
}

impl KvmBackend {
    /// Performs the operation against the sandbox this process is driving.
    pub(super) fn file_resident(
        &mut self,
        instance: &InstanceId,
        operation: &FileOperation,
    ) -> Result<FileAnswer, BackendFailureKind> {
        let Some(live) = self.live_for(instance) else {
            return Err(self.absent_kind(instance));
        };
        live.session.file(operation.clone()).map_err(failure_kind)
    }
}
