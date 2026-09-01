//! Turning the six file subcommands into the facade's filesystem request.
//!
//! The path is taken as operating system bytes and handed over unexamined. Whether it is a path
//! the sandbox will admit is the guest's decision, taken inside the guest, and a command line
//! that pre-approved one would be deciding something it cannot see.

use std::os::unix::ffi::OsStringExt as _;

use soma::{FileMachineRequest, FileOperation, InstanceId};

use crate::cli::{FileArgs, FileCommand, FilePathArgs};

use super::{PreparedOperation, RequestError, operation_id};

pub(super) fn prepare(arguments: FileArgs) -> Result<PreparedOperation, RequestError> {
    let (target, operation) = match arguments.command {
        FileCommand::Read(target) => {
            let path = path(&target);
            (target, FileOperation::Read { path })
        }
        FileCommand::Mkdir(target) => {
            let path = path(&target);
            (target, FileOperation::MakeDirectory { path })
        }
        FileCommand::List(target) => {
            let path = path(&target);
            (target, FileOperation::ReadDirectory { path })
        }
        FileCommand::Exists(target) => {
            let path = path(&target);
            (target, FileOperation::Exists { path })
        }
        FileCommand::Remove(arguments) => {
            let path = path(&arguments.target);
            (
                arguments.target,
                FileOperation::Remove {
                    path,
                    recursive: arguments.recursive,
                },
            )
        }
        FileCommand::Write(arguments) => {
            let path = path(&arguments.target);
            let bytes = read_host_file(&arguments.content_file)?;
            (arguments.target, FileOperation::Write { path, bytes })
        }
    };
    soma::check_guest_path(operation.path()).map_err(|_| RequestError::Path)?;
    let instance_id = InstanceId::new(target.instance_id).map_err(|_| RequestError::Identity)?;
    Ok(PreparedOperation::File {
        request: FileMachineRequest::new(
            operation_id(target.operation_id)?,
            instance_id.clone(),
            operation,
        ),
        instance_id,
    })
}

/// The path exactly as the operating system gave it.
fn path(target: &FilePathArgs) -> Vec<u8> {
    target.path.clone().into_vec()
}

/// Reads the host file a write takes its contents from, refusing one this call will not move.
///
/// The size is checked before the read rather than after it, so a caller that named a file far
/// larger than one transfer holds is refused rather than having it read into memory first.
fn read_host_file(path: &std::path::Path) -> Result<Vec<u8>, RequestError> {
    let metadata = std::fs::metadata(path).map_err(|_| RequestError::Content)?;
    if metadata.len() > soma::MAX_FILE_BYTES as u64 {
        return Err(RequestError::ContentTooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| RequestError::Content)?;
    if bytes.len() > soma::MAX_FILE_BYTES {
        // The file grew between the two calls. Refusing is the only honest answer: a shortened
        // write would put contents in the sandbox that match no file the caller ever had.
        return Err(RequestError::ContentTooLarge);
    }
    Ok(bytes)
}
