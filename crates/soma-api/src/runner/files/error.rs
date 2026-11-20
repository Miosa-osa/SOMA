//! The refusals a file call answers with, in the platform's file-route body.
//!
//! The platform's file actions answer a refusal as `{"code": ..., "error": ...}`: a stable code a
//! client branches on and a sentence a person reads. Every refusal here keeps that shape and the
//! platform's code wherever the platform has one for the same cause. A cause the guest reported
//! that the platform folds into a bare outage is named instead, because the guest was reached and
//! said why, and a retry would only be told the same thing.
//!
//! The sandbox itself being absent, stopped, or unreachable is not a file refusal. Those answers
//! are the runner's own and are the same on every route, so a client reads one sandbox-level
//! answer whatever it asked the sandbox to do.

use serde::Serialize;
use soma::FileRefusal;

/// One refused file call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileError {
    pub(crate) status: u16,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

#[derive(Serialize)]
struct Body<'a> {
    code: &'a str,
    error: &'a str,
}

impl FileError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn body(&self) -> Vec<u8> {
        crate::runner::public_wire::encode(&Body {
            code: self.code,
            error: &self.message,
        })
    }

    pub(crate) fn missing_path() -> Self {
        Self::new(400, "MISSING_PATH", "path is required")
    }

    pub(crate) fn invalid_path() -> Self {
        Self::new(400, "INVALID_PATH", "invalid path")
    }

    pub(crate) fn missing_paths() -> Self {
        Self::new(400, "MISSING_PARAMS", "from and to are required")
    }

    pub(crate) fn missing_mode() -> Self {
        Self::new(400, "MISSING_MODE", "mode is required")
    }

    pub(crate) fn invalid_mode() -> Self {
        Self::new(400, "INVALID_MODE", "mode must be octal such as 0644")
    }

    /// A mode with a set-user, set-group, or sticky bit, which the guest does not set.
    pub(crate) fn special_mode_bits() -> Self {
        Self::new(
            400,
            "INVALID_MODE",
            "the set-user, set-group and sticky bits cannot be set through the file API",
        )
    }

    pub(crate) fn invalid_body(message: &str) -> Self {
        Self::new(400, "INVALID_BODY", message)
    }

    pub(crate) fn invalid_query() -> Self {
        Self::new(
            400,
            "INVALID_QUERY",
            "the query string is not valid percent-encoded text",
        )
    }

    pub(crate) fn invalid_range() -> Self {
        Self::new(
            400,
            "INVALID_RANGE",
            "tail_bytes, offset and length must be non-negative integers",
        )
    }

    /// A file larger than the route moves; `limit` names the bound and `instead` the route that
    /// moves a file of any size the contract allows.
    pub(crate) fn too_large(limit: &str, instead: &str) -> Self {
        Self::new(
            413,
            "FILE_TOO_LARGE",
            format!("file exceeds {limit} limit; use {instead}"),
        )
    }

    /// A platform route this runner does not serve.
    pub(crate) fn unserved(route: &str) -> Self {
        Self::new(
            501,
            "FILE_ROUTE_UNSUPPORTED",
            format!("the SOMA runner does not serve {route}"),
        )
    }

    /// The guest was reached and declined `operation`, for the cause it named.
    pub(crate) fn refused(operation: &str, refusal: FileRefusal) -> Self {
        match refusal {
            FileRefusal::NotFound => Self::new(404, "NOT_FOUND", "path not found"),
            FileRefusal::Denied => Self::new(
                403,
                "PERMISSION_DENIED",
                format!("{operation} failed: permission denied"),
            ),
            FileRefusal::WrongKind => Self::new(
                422,
                "FS_OPERATION_FAILED",
                format!(
                    "{operation} failed: a directory was named where a file was required, or the reverse"
                ),
            ),
            FileRefusal::Exists => Self::new(
                409,
                "ALREADY_EXISTS",
                format!("{operation} failed: the path already exists"),
            ),
            FileRefusal::NotEmpty => Self::new(
                422,
                "FS_OPERATION_FAILED",
                format!("{operation} failed: the directory is not empty"),
            ),
            FileRefusal::TooLarge => Self::new(
                413,
                "FILE_TOO_LARGE",
                format!("{operation} failed: the file is larger than one transfer moves"),
            ),
            FileRefusal::Unsupported => Self::new(
                501,
                "FILE_OPERATION_UNSUPPORTED",
                format!("{operation} failed: the guest in this sandbox does not offer it"),
            ),
            FileRefusal::Failed => {
                Self::new(422, "FS_OPERATION_FAILED", format!("{operation} failed"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use soma::FileRefusal;

    use super::FileError;

    #[test]
    fn the_body_is_the_platform_file_route_shape() {
        assert_eq!(
            FileError::invalid_path().body(),
            br#"{"code":"INVALID_PATH","error":"invalid path"}"#.to_vec()
        );
    }

    #[test]
    fn every_guest_refusal_has_its_own_answer() {
        let answers = [
            (FileRefusal::NotFound, 404, "NOT_FOUND"),
            (FileRefusal::Denied, 403, "PERMISSION_DENIED"),
            (FileRefusal::WrongKind, 422, "FS_OPERATION_FAILED"),
            (FileRefusal::Exists, 409, "ALREADY_EXISTS"),
            (FileRefusal::NotEmpty, 422, "FS_OPERATION_FAILED"),
            (FileRefusal::TooLarge, 413, "FILE_TOO_LARGE"),
            (FileRefusal::Unsupported, 501, "FILE_OPERATION_UNSUPPORTED"),
            (FileRefusal::Failed, 422, "FS_OPERATION_FAILED"),
        ];
        for (refusal, status, code) in answers {
            let error = FileError::refused("rename", refusal);
            assert_eq!((error.status, error.code), (status, code), "{refusal:?}");
        }
    }
}
