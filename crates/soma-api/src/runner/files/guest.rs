//! The guest file operations one platform file call is made of.
//!
//! Most platform calls are one guest operation. A few are a short sequence: a write lands in a
//! temporary sibling and is renamed over its target, so a reader never sees half a file; a read
//! of a file larger than one transfer is several ranged reads; a rename into an existing
//! directory moves the source inside it, as `mv` does. The sequences live here, behind one small
//! seam, so they run the same against a real sandbox and against the in-memory guest the tests
//! drive.

<<<<<<< HEAD
mod staged;
=======
>>>>>>> origin/r3/soma-unified
mod transfer;

use soma::{
    FileAnswer, FileEntry, FileKind, FileMachineRequest, FileOperation, FileRefusal, InstanceId,
    ManagedFailure,
};

use crate::facade::SandboxFacade;

use super::path::GuestPath;

<<<<<<< HEAD
pub(crate) use staged::Staged;
pub(crate) use transfer::{Slice, Written, read_range, read_slice, read_whole, write};
=======
pub(crate) use transfer::{Slice, Written, read_slice, read_whole, write};
>>>>>>> origin/r3/soma-unified

/// What one sandbox's guest answers to one file operation.
pub(crate) trait GuestFiles {
    /// Performs one operation.
    ///
    /// # Errors
    ///
    /// Returns the facade's failure when the sandbox could not be reached at all; a cause the
    /// guest reported is an answer, [`FileAnswer::Refused`].
    fn perform(&mut self, operation: FileOperation) -> Result<FileAnswer, ManagedFailure>;
}

/// One sandbox's guest, reached through the facade the runner leased for this call.
pub(crate) struct OnFacade<'a> {
    pub(crate) facade: &'a mut dyn SandboxFacade,
    pub(crate) instance_id: &'a InstanceId,
}

impl GuestFiles for OnFacade<'_> {
    fn perform(&mut self, operation: FileOperation) -> Result<FileAnswer, ManagedFailure> {
        // A generated identity is always one the facade accepts; were it ever refused, the call
        // would not have reached the guest, which is what an unavailable backend means.
        let operation_id = crate::wire::operation_id(None)
            .map_err(|_| ManagedFailure::Backend(soma::BackendFailureKind::Unavailable))?;
        self.facade
            .file(FileMachineRequest::new(
                operation_id,
                self.instance_id.clone(),
                operation,
            ))
            .map(|outcome| outcome.answer)
    }
}

/// Why a file call stopped before it finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The guest was reached and declined, for the cause it named.
    Refused(FileRefusal),
    /// The sandbox could not be reached through the facade.
    Failed(ManagedFailure),
    /// The guest answered with something this operation never answers with.
    Unexpected,
}

/// What a stat reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stat {
    pub(crate) kind: FileKind,
    pub(crate) size: u64,
    pub(crate) mode: u32,
    pub(crate) modified: u64,
}

/// Asks once, turning a refusal into a stop so a sequence can use `?`.
pub(crate) fn ask(
    guest: &mut dyn GuestFiles,
    operation: FileOperation,
) -> Result<FileAnswer, Stop> {
    match guest.perform(operation) {
        Ok(FileAnswer::Refused(refusal)) => Err(Stop::Refused(refusal)),
        Ok(answer) => Ok(answer),
        Err(failure) => Err(Stop::Failed(failure)),
    }
}

fn done(guest: &mut dyn GuestFiles, operation: FileOperation) -> Result<(), Stop> {
    match ask(guest, operation)? {
        FileAnswer::Done => Ok(()),
        _ => Err(Stop::Unexpected),
    }
}

pub(crate) fn list(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
) -> Result<(Vec<FileEntry>, bool), Stop> {
    match ask(guest, FileOperation::ReadDirectory { path: bytes(path) })? {
        FileAnswer::Listed { entries, more } => Ok((entries, more)),
        _ => Err(Stop::Unexpected),
    }
}

pub(crate) fn stat(guest: &mut dyn GuestFiles, path: &GuestPath) -> Result<Stat, Stop> {
    match ask(guest, FileOperation::Stat { path: bytes(path) })? {
        FileAnswer::Stat {
            kind,
            size,
            mode,
            modified,
        } => Ok(Stat {
            kind,
            size,
            mode,
            modified,
        }),
        _ => Err(Stop::Unexpected),
    }
}

/// What is at the path, or `None` when nothing is.
pub(crate) fn kind_of(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
) -> Result<Option<FileKind>, Stop> {
    match ask(guest, FileOperation::Exists { path: bytes(path) })? {
        FileAnswer::Status { kind } => Ok(kind),
        _ => Err(Stop::Unexpected),
    }
}

/// Makes the directory and every missing parent; an existing directory is success.
pub(crate) fn make_directory(guest: &mut dyn GuestFiles, path: &GuestPath) -> Result<(), Stop> {
    done(guest, FileOperation::MakeDirectory { path: bytes(path) })
}

/// Removes the path; a path that is already absent is removed, as `rm -f` has it.
pub(crate) fn remove(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    recursive: bool,
) -> Result<(), Stop> {
    match done(
        guest,
        FileOperation::Remove {
            path: bytes(path),
            recursive,
        },
    ) {
        Err(Stop::Refused(FileRefusal::NotFound)) => Ok(()),
        other => other,
    }
}

pub(crate) fn set_mode(
    guest: &mut dyn GuestFiles,
    path: &GuestPath,
    mode: u32,
) -> Result<(), Stop> {
    done(
        guest,
        FileOperation::SetMode {
            path: bytes(path),
            mode,
        },
    )
}

/// Moves `from` to `to` the way the platform's `mkdir -p` and `mv` do.
///
/// The destination's directory is made first. A destination that is an existing directory
/// receives the source inside it, under the source's own name; any other destination is
/// replaced atomically.
pub(crate) fn move_to(
    guest: &mut dyn GuestFiles,
    from: &GuestPath,
    to: &GuestPath,
) -> Result<(), Stop> {
    make_directory(guest, &to.parent())?;
    let target = if kind_of(guest, to)? == Some(FileKind::Directory) {
        to.join(from.file_name())
            .map_err(|_| Stop::Refused(FileRefusal::Failed))?
    } else {
        to.clone()
    };
    rename(guest, from, &target)
}

/// One atomic rename, with nothing made or looked at first.
fn rename(guest: &mut dyn GuestFiles, from: &GuestPath, to: &GuestPath) -> Result<(), Stop> {
    done(
        guest,
        FileOperation::Rename {
            path: bytes(from),
            to: bytes(to),
        },
    )
}

fn bytes(path: &GuestPath) -> Vec<u8> {
    path.as_bytes().to_vec()
}
