//! The operation named in the last segment of a filesystem or terminal path.

use super::{FilesystemOperation, TerminalOperation};
use crate::envelope::ApiError;

pub(super) fn filesystem_operation(operation: &str) -> Result<FilesystemOperation, ApiError> {
    match operation {
        "read" => Ok(FilesystemOperation::Read),
        "write" => Ok(FilesystemOperation::Write),
        "list" => Ok(FilesystemOperation::List),
        "exists" => Ok(FilesystemOperation::Exists),
        "remove" => Ok(FilesystemOperation::Remove),
        "mkdir" => Ok(FilesystemOperation::MakeDirectory),
        "stat" => Ok(FilesystemOperation::Stat),
        "rename" => Ok(FilesystemOperation::Rename),
        "read_range" => Ok(FilesystemOperation::ReadRange),
        "write_range" => Ok(FilesystemOperation::WriteRange),
        "chmod" => Ok(FilesystemOperation::Chmod),
        _ => Err(ApiError::not_found()),
    }
}

pub(super) fn terminal_operation(operation: &str) -> Result<TerminalOperation, ApiError> {
    match operation {
        "open" => Ok(TerminalOperation::Open),
        "write" => Ok(TerminalOperation::Write),
        "read" => Ok(TerminalOperation::Read),
        "resize" => Ok(TerminalOperation::Resize),
        "close" => Ok(TerminalOperation::Close),
        _ => Err(ApiError::not_found()),
    }
}
